//! Byte-accounting totality for [`gamut_png::deconstruct`] (issue #224): every PNG's segments
//! must tile `0..len` exactly, and the reported figures must match what the file actually holds.
//!
//! The fixtures come from **libpng**, not from gamut's encoder, wherever the claim is about
//! reading a foreign file: interlaced streams, forced filters and sub-byte depths are all things
//! `gamut_png::PngEncoder` cannot write, so a gamut-only corpus could not reach them, and a
//! filter histogram checked against gamut's own choice would be self-consistent rather than
//! correct.

mod common;

use std::time::Instant;

use gamut_core::{Dimensions, EncodeImage, ImageRef, Rgb8, Rgba8};
use gamut_png::{
    ChunkStats, FilterStrategy, FilterType, PngEncoder, Segment, SegmentKind, deconstruct,
};

/// Folds over the segments asserting: non-empty, first starts at 0, each end chains to the next
/// start (contiguous, non-overlapping), and the last ends at `len` — the every-byte invariant.
/// Deliberately re-derived here rather than trusting `is_fully_classified`, which is the thing
/// under test.
fn assert_covers(segments: &[Segment], len: usize) {
    assert!(!segments.is_empty(), "at least one segment");
    assert_eq!(segments[0].range.start, 0, "coverage starts at 0");
    for pair in segments.windows(2) {
        assert_eq!(
            pair[0].range.end, pair[1].range.start,
            "segments are contiguous and non-overlapping"
        );
    }
    assert_eq!(
        segments.last().expect("non-empty").range.end,
        len,
        "coverage runs to end of file"
    );
    for s in segments {
        assert!(s.range.end > s.range.start, "no empty segment: {s:?}");
    }
}

/// A deterministic RGB pattern with enough local structure that filters differ between rows.
fn rgb(w: u32, h: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            out.push((x ^ y) as u8);
            out.push(x.wrapping_mul(3).wrapping_add(y) as u8);
            out.push(x.wrapping_add(y.wrapping_mul(7)) as u8);
        }
    }
    out
}

fn encode_rgb(w: u32, h: u32) -> Vec<u8> {
    let src = rgb(w, h);
    let dims = Dimensions::new(w, h).expect("valid dimensions");
    let image = ImageRef::<Rgb8>::new(&src, dims).expect("buffer matches dimensions");
    let mut png = Vec::new();
    PngEncoder::new()
        .encode_image(image, &mut png)
        .expect("encode");
    png
}

#[test]
fn segments_tile_every_byte_of_a_gamut_encode() {
    for (w, h) in [(1, 1), (17, 13), (64, 40)] {
        let png = encode_rgb(w, h);
        let report = deconstruct(&png).expect("deconstruct");
        assert_covers(&report.segments, png.len());
        assert!(report.is_fully_classified(), "{report:?}");
        assert!(report.is_intact(), "{report:?}");
        assert_eq!(report.file_len, png.len());
    }
}

#[test]
fn segments_tile_every_byte_of_every_libpng_colour_type_and_depth() {
    for &(color_type, depth) in common::TABLE_12 {
        for interlace in [false, true] {
            let png = common::libpng_fixture(17, 13, color_type, depth, interlace);
            let report = deconstruct(&png).unwrap_or_else(|e| {
                panic!("deconstruct ct={color_type} depth={depth} interlace={interlace}: {e:?}")
            });
            assert_covers(&report.segments, png.len());
            assert!(
                report.is_intact(),
                "ct={color_type} depth={depth} interlace={interlace}: {report:?}"
            );
            assert_eq!(report.header.bit_depth, depth);
            assert_eq!(report.header.interlaced, interlace);
        }
    }
}

#[test]
fn chunk_totals_match_an_independent_scan() {
    let png = encode_rgb(40, 30);
    let report = deconstruct(&png).expect("deconstruct");

    // A naive second scan written here, so a defect in the walk's accumulation cannot agree with
    // itself. 8 signature bytes, then `length || type || data || crc`.
    let mut at = 8usize;
    let mut seen: Vec<([u8; 4], usize, usize)> = Vec::new();
    while at + 12 <= png.len() {
        let len = u32::from_be_bytes([png[at], png[at + 1], png[at + 2], png[at + 3]]) as usize;
        let ty = [png[at + 4], png[at + 5], png[at + 6], png[at + 7]];
        match seen.iter_mut().find(|(t, _, _)| *t == ty) {
            Some(entry) => {
                entry.1 += 1;
                entry.2 += len;
            }
            None => seen.push((ty, 1, len)),
        }
        at += 12 + len;
    }
    assert_eq!(at, png.len(), "the naive scan must consume the file too");

    let got: Vec<_> = report
        .chunks
        .iter()
        .map(|c| (c.chunk_type, c.count, c.payload_bytes))
        .collect();
    assert_eq!(got, seen, "chunk table, in first-appearance order");

    // Signature + every chunk's payload and framing is the whole file.
    let total: usize = report.chunks.iter().map(ChunkStats::total_bytes).sum();
    assert_eq!(total + 8, png.len());
    assert_eq!(report.framing_bytes(), report.chunks.len() * 12);
}

/// A chunk type that appears more than once must accumulate, not overwrite.
///
/// Every other fixture here carries at most one chunk of each type, so the accumulate arm of the
/// chunk table never ran: `count` stayed at the 1 it is inserted with and `payload_bytes` at the
/// first chunk's length, and no assertion could tell.
#[test]
fn repeated_chunk_types_accumulate_count_and_payload() {
    let first: &[u8] = b"Author\0alice";
    let second: &[u8] = b"Comment\0a considerably longer comment";
    let png = common::png_from_chunks(&[
        common::chunk(b"IHDR", &common::ihdr_payload(4, 4, 8, 2, 0)),
        common::chunk(b"tEXt", first),
        common::chunk(b"tEXt", second),
        common::chunk(b"IDAT", &common::zlib(&[0u8; 4 * (4 * 3 + 1)])),
        common::chunk(b"IEND", &[]),
    ]);

    let report = deconstruct(&png).expect("deconstruct");
    assert_covers(&report.segments, png.len());

    let text = report.chunk(b"tEXt").expect("tEXt accounted");
    assert_eq!(text.count, 2, "both chunks counted");
    assert_eq!(
        text.payload_bytes,
        first.len() + second.len(),
        "payloads summed, not overwritten"
    );
    assert_eq!(text.framing_bytes(), 24, "12 framing bytes per chunk");
    assert_eq!(text.total_bytes(), first.len() + second.len() + 24);
    // The table lists each type once, in first-appearance order.
    assert_eq!(
        report
            .chunks
            .iter()
            .map(|c| c.chunk_type)
            .collect::<Vec<_>>(),
        vec![*b"IHDR", *b"tEXt", *b"IDAT", *b"IEND"]
    );
}

/// A stream large enough to split across several IDAT chunks: the same accumulation, on the path
/// that actually produces it in production rather than a hand-built file.
/// A synthetic chunk type for the quadratic regression fixture below: four lowercase letters, so
/// it is ancillary, private, and can never collide with `IHDR`, `IDAT` or `IEND`. 26⁴ = 456 976
/// distinct types, comfortably more than the fixture uses.
fn synthetic_type(i: usize) -> [u8; 4] {
    [
        b'a' + (i % 26) as u8,
        b'a' + (i / 26 % 26) as u8,
        b'a' + (i / 676 % 26) as u8,
        b'a' + (i / 17_576 % 26) as u8,
    ]
}

/// Deconstruction must not slow down when every chunk type in the file is distinct.
///
/// A chunk type is four unvalidated bytes and the walk never drops a chunk, so an attacker
/// chooses how many *distinct* types a file carries — one per 12-byte chunk, if they like.
/// Accumulating the per-type totals with a linear scan made this quadratic in the file length
/// (measured: 4.8 MB → 40.9 s), reachable from `gamut inspect` on an untrusted file.
///
/// The claim asserted is not "fast" — an absolute wall-clock ceiling is flaky under `llvm-cov`
/// and parallel test binaries — but "the cost does not depend on how many distinct types the file
/// carries". The two halves are byte-for-byte the same length and carry the same number of
/// chunks, differing only in how many types those chunks use, and they run back to back in one
/// process under one load, so each calibrates the other. The fixed path measures ~2–4×; the
/// defect is three orders of magnitude worse, leaving ~5× of headroom above the fix and ~50×
/// below the defect. The structural assertions below mean it is not purely a timing test.
#[test]
fn the_chunk_tally_does_not_slow_down_when_every_type_is_distinct() {
    /// Empty chunks between IHDR and IEND: 12 bytes each, so ~3.1 MB per half.
    const CHUNKS: usize = 262_144;

    let build = |distinct: bool| {
        let mut framed = Vec::with_capacity(CHUNKS + 2);
        framed.push(common::chunk(b"IHDR", &common::ihdr_payload(1, 1, 8, 2, 0)));
        framed.extend(
            (0..CHUNKS).map(|i| common::chunk(&synthetic_type(if distinct { i } else { 0 }), &[])),
        );
        framed.push(common::chunk(b"IEND", &[]));
        common::png_from_chunks(&framed)
    };
    let repeated = build(false);
    let distinct = build(true);
    assert_eq!(
        repeated.len(),
        distinct.len(),
        "the two halves must be the same length, or the ratio compares two workloads"
    );

    let started = Instant::now();
    let repeated_report = deconstruct(&repeated).expect("deconstruct");
    let repeated_elapsed = started.elapsed();
    let started = Instant::now();
    let distinct_report = deconstruct(&distinct).expect("deconstruct");
    let distinct_elapsed = started.elapsed();

    assert_eq!(
        distinct_report.chunks.len(),
        CHUNKS + 2,
        "IHDR, one entry per distinct type, IEND"
    );
    assert!(
        distinct_report.chunks.iter().all(|stats| stats.count == 1),
        "every synthetic type appears exactly once"
    );
    assert_eq!(
        repeated_report.chunks.len(),
        3,
        "IHDR, the one repeated type, IEND"
    );
    assert_eq!(repeated_report.chunks[1].count, CHUNKS);

    assert!(
        distinct_elapsed < 20 * repeated_elapsed,
        "distinct types cost {distinct_elapsed:?} against {repeated_elapsed:?} for the same \
         bytes with one type: the tally is scaling with the number of distinct types"
    );
}

#[test]
fn a_multi_idat_encode_accumulates_every_idat() {
    // Incompressible, so the zlib stream stays far above the 64 KiB per-chunk cap.
    let (w, h) = (256u32, 256u32);
    let src = common::corpus::noise_rgb(w);
    let dims = Dimensions::new(w, h).expect("valid dimensions");
    let image = ImageRef::<Rgb8>::new(&src, dims).expect("buffer matches dimensions");
    let mut png = Vec::new();
    PngEncoder::new()
        .encode_image(image, &mut png)
        .expect("encode");

    let report = deconstruct(&png).expect("deconstruct");
    assert_covers(&report.segments, png.len());
    let idat = report.chunk(b"IDAT").expect("IDAT accounted");
    assert!(idat.count > 1, "the fixture must actually split: {idat:?}");
    assert_eq!(
        idat.payload_bytes, report.idat_compressed,
        "the chunk table and the compressed total are the same bytes counted twice"
    );
    assert!(report.is_intact(), "{report:?}");
}

#[test]
fn trailing_bytes_after_iend_are_a_trailer() {
    let mut png = encode_rgb(8, 8);
    let clean = png.len();
    png.extend_from_slice(b"junk after the datastream");
    let report = deconstruct(&png).expect("deconstruct");

    assert_covers(&report.segments, png.len());
    let last = report.segments.last().expect("non-empty");
    assert_eq!(last.kind, SegmentKind::Trailer);
    assert_eq!(last.range, clean..png.len());
    // A trailer is not damage the walk failed to classify, but the file is not pristine.
    assert!(report.is_fully_classified());
    assert!(!report.is_intact());
}

#[test]
fn a_truncated_tail_is_reported_not_an_error() {
    let full = encode_rgb(24, 24);
    // Cut inside the IDAT payload: the chunk header frames, but its data overruns the input.
    let png = &full[..full.len() - 20];
    let report = deconstruct(png).expect("a truncated file still has a header to report on");

    assert_covers(&report.segments, png.len());
    assert_eq!(
        report.segments.last().expect("non-empty").kind,
        SegmentKind::Truncated
    );
    assert!(report.is_fully_classified());
    assert!(!report.is_intact(), "truncation is not intact");
    // Everything derived from IHDR survives the damage — that is the point of the split.
    assert_eq!(report.header.width, 24);
    assert!(report.filtered_len > 0);
}

#[test]
fn unknown_ancillary_and_critical_chunks_are_accounted() {
    let extra: [([u8; 4], &[u8]); 2] = [(*b"abCd", &[1, 2, 3]), (*b"ABCD", &[4])];
    let png = common::libpng_with_extra_chunks(12, 9, &extra);
    let report = deconstruct(&png).expect("an unknown critical chunk is reported, not an error");

    assert_covers(&report.segments, png.len());
    let ancillary = report.chunk(b"abCd").expect("unknown ancillary accounted");
    let critical = report.chunk(b"ABCD").expect("unknown critical accounted");
    assert_eq!(ancillary.payload_bytes, 3);
    assert_eq!(critical.payload_bytes, 1);
    assert!(
        ancillary.is_ancillary(),
        "lowercase first byte is ancillary"
    );
    assert!(!critical.is_ancillary(), "uppercase first byte is critical");
}

#[test]
fn a_crc_mismatch_is_flagged_not_fatal() {
    let mut png = encode_rgb(8, 8);
    // Corrupt IEND's stored CRC, not any payload: every chunk still frames and IHDR still parses,
    // so the only thing wrong with the file is a checksum. Corrupting a payload instead would
    // make IHDR unparsable, which is a hard error and a different claim.
    let last = png.len() - 1;
    png[last] ^= 0xFF;
    let report = deconstruct(&png).expect("a CRC mismatch is reported, not an error");

    assert_covers(&report.segments, png.len());
    let bad = report
        .segments
        .iter()
        .filter_map(|s| match s.kind {
            SegmentKind::Chunk {
                chunk_type,
                crc_ok: false,
                ..
            } => Some(chunk_type),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(bad, vec![*b"IEND"], "exactly the damaged chunk is flagged");
    assert!(report.is_fully_classified());
    assert!(!report.is_intact(), "a bad CRC is not intact");
}

#[test]
fn the_filter_histogram_matches_the_filter_libpng_was_forced_to_use() {
    // libpng, not gamut, picks the filters here, so this cannot be satisfied by a self-consistent
    // round trip: it is the differential half of the report's claim.
    let forced = [
        (libpng_oracle::FILTER_NONE, FilterType::None),
        (libpng_oracle::FILTER_SUB, FilterType::Sub),
        (libpng_oracle::FILTER_UP, FilterType::Up),
        (libpng_oracle::FILTER_AVG, FilterType::Average),
        (libpng_oracle::FILTER_PAETH, FilterType::Paeth),
    ];
    for (mask, expected) in &forced {
        let png = common::libpng_forced_filter(20, 14, *mask);
        let report = deconstruct(&png).expect("deconstruct");
        let filters = report
            .filters
            .expect("a sound IDAT stream yields a histogram");

        assert_eq!(filters.total(), 14, "one filter byte per scanline");
        assert_eq!(
            filters.count(*expected),
            14,
            "every row used {expected:?} (mask {mask:#04x})"
        );
    }
}

/// The histogram must advance one scanline at a time.
///
/// Every other histogram assertion here forces a single filter for the whole image, which cannot
/// tell a correct per-row walk from one that re-reads the same byte: both report `height` of the
/// one filter. This fixture's rows choose differently, so a stalled cursor collapses the
/// distribution to a single bucket and is visible.
#[test]
fn the_histogram_walks_each_scanline_not_the_first_one_repeatedly() {
    const SIDE: u32 = 64;
    let src = common::corpus::sprite_rgba(SIDE);
    let dims = Dimensions::new(SIDE, SIDE).expect("valid dimensions");
    let image = ImageRef::<Rgba8>::new(&src, dims).expect("buffer matches dimensions");
    let mut png = Vec::new();
    PngEncoder::new()
        .with_filter(FilterStrategy::MinSumAbs)
        .encode_image(image, &mut png)
        .expect("encode");

    let report = deconstruct(&png).expect("deconstruct");
    let h = report.filters.expect("sound stream");
    assert_eq!(h.total(), SIDE, "one filter byte per scanline");

    let used = [
        FilterType::None,
        FilterType::Sub,
        FilterType::Up,
        FilterType::Average,
        FilterType::Paeth,
    ]
    .into_iter()
    .filter(|&f| h.count(f) > 0)
    .count();
    assert!(
        used >= 2,
        "this fixture's rows must not all choose the same filter, got {used} distinct"
    );
}

#[test]
fn interlaced_filtered_length_is_the_per_pass_sum() {
    // 5x3 and 1x1 leave several Adam7 passes empty; an empty pass contributes no bytes at all,
    // not even a filter byte (§7.3).
    for (w, h) in [(1, 1), (5, 3), (17, 13)] {
        let png = common::libpng_fixture(w, h, libpng_oracle::COLOR_RGB, 8, true);
        let report = deconstruct(&png).expect("deconstruct");

        let summed: usize = report.passes.iter().map(|p| p.filtered_len).sum();
        assert_eq!(
            summed, report.filtered_len,
            "{w}x{h}: passes sum to the whole"
        );
        assert!(
            report.passes.iter().all(|p| p.width > 0 && p.height > 0),
            "empty passes are omitted, not zero-sized: {:?}",
            report.passes
        );

        let rows: u32 = report.passes.iter().map(|p| p.height).sum();
        assert_eq!(
            report.filters.expect("sound stream").total(),
            rows,
            "{w}x{h}: one filter byte per scanline of every non-empty pass"
        );
    }
}

#[test]
fn sub_byte_row_padding_is_counted() {
    // 5 pixels at depth 4 is 20 bits, which pads to 3 bytes per row -- `div_ceil`, not `/`.
    let png = common::libpng_fixture(5, 3, libpng_oracle::COLOR_GRAY, 4, false);
    let report = deconstruct(&png).expect("deconstruct");

    assert_eq!(report.passes.len(), 1, "not interlaced");
    assert_eq!(report.passes[0].row_bytes, 3);
    assert_eq!(report.filtered_len, 3 * (3 + 1));
}

#[test]
fn a_corrupt_zlib_stream_with_a_valid_crc_yields_no_histogram() {
    // The only fixture that falsifies `is_intact`'s `filters.is_some()` conjunct on its own:
    // framing is perfect, every CRC is valid, and only the compressed payload is nonsense.
    let png = common::png_with_garbage_idat(16, 8);
    let report = deconstruct(&png).expect("a corrupt IDAT is reported, not an error");

    assert_covers(&report.segments, png.len());
    assert!(report.is_fully_classified());
    assert!(
        report.segments.iter().all(|s| match s.kind {
            SegmentKind::Chunk { crc_ok, .. } => crc_ok,
            _ => true,
        }),
        "every CRC is valid in this fixture"
    );
    assert_eq!(report.filters, None, "the histogram is the only casualty");
    assert!(!report.is_intact());
    // Framing- and IHDR-derived figures are unaffected.
    assert_eq!(report.header.width, 16);
    assert!(report.idat_compressed > 0);
    assert!(report.filtered_len > 0);
}

#[test]
fn an_over_budget_image_reports_everything_but_the_histogram() {
    // A hand-built IHDR claiming 2^30 x 2^30 with a tiny IDAT: the filtered stream it implies is
    // far past the inflation cap, so the walk must decline to inflate rather than try. Without
    // this the cap comparison is never exercised.
    let png = common::png_with_huge_ihdr();
    let report = deconstruct(&png).expect("an oversized header is reported, not an error");

    assert_covers(&report.segments, png.len());
    assert_eq!(report.filters, None, "declined: over the inflation cap");
    assert!(
        report.filtered_len > (64 << 20),
        "the implied stream is huge"
    );
    assert_eq!(report.header.width, 1 << 30);
}

#[test]
fn a_file_with_no_header_to_report_on_is_an_error() {
    assert!(deconstruct(&[]).is_err(), "empty input");
    assert!(deconstruct(b"not a png at all").is_err(), "bad signature");

    let signature_only = common::SIGNATURE.to_vec();
    assert!(deconstruct(&signature_only).is_err(), "no chunk at all");

    let mut first_not_ihdr = common::SIGNATURE.to_vec();
    first_not_ihdr.extend_from_slice(&common::chunk(b"gAMA", &45455u32.to_be_bytes()));
    assert!(
        deconstruct(&first_not_ihdr).is_err(),
        "first chunk is not IHDR"
    );

    let mut bad_ihdr = common::SIGNATURE.to_vec();
    bad_ihdr.extend_from_slice(&common::chunk(b"IHDR", &[0u8; 13]));
    assert!(deconstruct(&bad_ihdr).is_err(), "zero dimensions in IHDR");
}

#[test]
fn the_derived_ratios_are_the_stated_quotients() {
    let png = encode_rgb(32, 24);
    let report = deconstruct(&png).expect("deconstruct");

    let pixels = f64::from(report.header.width) * f64::from(report.header.height);
    assert!(
        (report.bits_per_pixel() - (report.file_len as f64 * 8.0 / pixels)).abs() < 1e-9,
        "bits_per_pixel is the whole file over the pixel count"
    );
    assert!(
        (report.idat_ratio() - (report.idat_compressed as f64 / report.filtered_len as f64)).abs()
            < 1e-9,
        "idat_ratio is IDAT over the filtered stream"
    );
    assert_eq!(
        report.overhead_bytes(),
        report.file_len - report.idat_compressed
    );
    // A real photo-ish pattern must actually compress, or the fixture is not measuring anything.
    assert!(report.idat_ratio() < 1.0, "{}", report.idat_ratio());
}

#[test]
fn a_brute_force_encode_still_accounts_and_reports_its_filters() {
    // The strategy that costs the most and is most likely to trip an accounting assumption.
    let src = rgb(48, 32);
    let dims = Dimensions::new(48, 32).expect("valid dimensions");
    let image = ImageRef::<Rgb8>::new(&src, dims).expect("buffer matches dimensions");
    let mut png = Vec::new();
    PngEncoder::new()
        .with_filter(FilterStrategy::BruteForce)
        .encode_image(image, &mut png)
        .expect("encode");

    let report = deconstruct(&png).expect("deconstruct");
    assert_covers(&report.segments, png.len());
    assert!(report.is_intact());
    assert_eq!(report.filters.expect("sound stream").total(), 32);
}
