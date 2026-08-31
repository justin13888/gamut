//! Byte-accounting totality: every fixture's segments must tile `0..len` exactly, and the segment
//! kinds must match the expected decomposition (boxes, appended stream, trailer).

mod common;

use common::{bx, c2pa_box, cat, clean_file, ftyp, hvc1_item, jumbf_store};
use gamut_heic::{HeifContainer, SegmentKind};

/// Folds over the segments asserting: non-empty, first starts at 0, each end chains to the next
/// start (contiguous, non-overlapping), and the last ends at `len` — the every-byte invariant.
fn assert_covers(container: &HeifContainer, len: usize) {
    let segs = container.segments();
    assert!(!segs.is_empty(), "at least one segment");
    assert_eq!(segs[0].range.start, 0, "coverage starts at 0");
    for pair in segs.windows(2) {
        assert_eq!(
            pair[0].range.end, pair[1].range.start,
            "segments are contiguous and non-overlapping"
        );
    }
    assert_eq!(
        segs.last().unwrap().range.end,
        len,
        "coverage runs to end of file"
    );
    for s in segs {
        assert!(s.range.end > s.range.start, "no empty segment");
    }
}

/// The top-level box types, in order (the `SegmentKind::Box` segments).
fn box_types(container: &HeifContainer) -> Vec<[u8; 4]> {
    container.boxes().map(|(ty, _)| ty).collect()
}

#[test]
fn clean_file_is_all_boxes() {
    let data = clean_file(1, vec![hvc1_item(1, vec![0xAA, 0xBB, 0xCC, 0xDD])]);
    let c = HeifContainer::parse(&data).unwrap();

    assert_covers(&c, data.len());
    assert_eq!(box_types(&c), vec![*b"ftyp", *b"meta", *b"mdat"]);
    assert!(c.appended_stream().is_none());
    assert!(c.trailer().is_none());
    // Every segment is a Box.
    assert!(
        c.segments()
            .iter()
            .all(|s| matches!(s.kind, SegmentKind::Box { .. }))
    );

    // `data()` returns the exact input buffer it was parsed from (pins the accessor against the
    // `Vec::leak(..)` replacements). Compare by contents *and* pointer identity — it borrows `data`.
    assert_eq!(c.data(), data.as_slice());
    assert_eq!(c.data().as_ptr(), data.as_ptr());
    assert_eq!(c.data().len(), data.len());
}

#[test]
fn unknown_top_level_box_surfaces_verbatim() {
    // Google Motion Photo appends a top-level `mpvd` box (absent from MP4RA) after the image boxes.
    let clean = clean_file(1, vec![hvc1_item(1, vec![1, 2, 3, 4])]);
    let mpvd_body = b"MP4-motion-photo-video-data".to_vec();
    let data = cat(&[clean.clone(), bx(b"mpvd", &mpvd_body)]);
    let c = HeifContainer::parse(&data).unwrap();

    assert_covers(&c, data.len());
    assert_eq!(box_types(&c), vec![*b"ftyp", *b"meta", *b"mdat", *b"mpvd"]);
    // The unknown box is surfaced with its exact body — nothing dropped.
    let mpvd = c
        .boxes()
        .find(|(ty, _)| ty == b"mpvd")
        .expect("mpvd surfaced");
    assert_eq!(mpvd.1, mpvd_body.as_slice());
    // It is a primary-stream box, not an appended stream or trailer.
    assert!(c.appended_stream().is_none());
    assert!(c.trailer().is_none());
}

#[test]
fn alternate_size_headers_account_for_every_header_byte() {
    let clean = clean_file(1, vec![hvc1_item(1, vec![1, 2, 3, 4])]);
    let mut large = vec![0, 0, 0, 1, b'f', b'r', b'e', b'e'];
    large.extend_from_slice(&19_u64.to_be_bytes());
    large.extend_from_slice(&[0xAA, 0xBB, 0xCC]);
    let open = vec![0, 0, 0, 0, b's', b'k', b'i', b'p', 0xDD, 0xEE];
    let data = cat(&[clean.clone(), large.clone(), open.clone()]);
    let c = HeifContainer::parse(&data).unwrap();

    assert_covers(&c, data.len());
    assert_eq!(
        box_types(&c),
        vec![*b"ftyp", *b"meta", *b"mdat", *b"free", *b"skip"]
    );
    let free = c.boxes().find(|(ty, _)| ty == b"free").unwrap();
    assert_eq!(free.1, &[0xAA, 0xBB, 0xCC]);
    let skip = c.boxes().find(|(ty, _)| ty == b"skip").unwrap();
    assert_eq!(skip.1, &[0xDD, 0xEE]);
    assert_eq!(
        c.segments()[3].range,
        clean.len()..clean.len() + large.len()
    );
}

#[test]
fn second_ftyp_starts_appended_stream_to_eof() {
    // Samsung motion photo: a second whole file (second top-level `ftyp` + its own stream), then a
    // proprietary trailer — all opaque, all one appended segment starting at the second ftyp.
    let clean = clean_file(1, vec![hvc1_item(1, vec![9, 9, 9, 9])]);
    let appendix = cat(&[ftyp(b"mp42"), b"\xDE\xAD\xBE\xEFtrailing-garbage".to_vec()]);
    let data = cat(&[clean.clone(), appendix.clone()]);
    let c = HeifContainer::parse(&data).unwrap();

    assert_covers(&c, data.len());
    // Primary stream is unchanged; the appendix is a single AppendedStream.
    assert_eq!(box_types(&c), vec![*b"ftyp", *b"meta", *b"mdat"]);
    let appended = c.appended_stream().expect("appended stream");
    assert_eq!(appended, appendix.as_slice());
    // It starts exactly at the second ftyp (== end of the clean file) and runs to EOF.
    let appended_segment = c
        .segments()
        .iter()
        .find(|s| matches!(s.kind, SegmentKind::AppendedStream(_)))
        .unwrap();
    assert_eq!(appended_segment.range, clean.len()..data.len());
    assert!(c.trailer().is_none());
}

#[test]
fn trailing_garbage_without_ftyp_is_a_trailer() {
    // Non-box trailing bytes with no second ftyp: retained as an explicit Trailer.
    let clean = clean_file(1, vec![hvc1_item(1, vec![5, 5, 5, 5])]);
    let garbage = b"SamsungSEF\x00trailer-blob".to_vec();
    let data = cat(&[clean.clone(), garbage.clone()]);
    let c = HeifContainer::parse(&data).unwrap();

    assert_covers(&c, data.len());
    assert_eq!(box_types(&c), vec![*b"ftyp", *b"meta", *b"mdat"]);
    let trailer = c.trailer().expect("trailer");
    assert_eq!(trailer, garbage.as_slice());
    let trailer_segment = c
        .segments()
        .iter()
        .find(|s| matches!(s.kind, SegmentKind::Trailer(_)))
        .unwrap();
    assert_eq!(trailer_segment.range, clean.len()..data.len());
    assert!(c.appended_stream().is_none());
}

#[test]
fn malformed_box_before_meta_is_a_parse_error() {
    // A malformed trailing box is only tolerated once ftyp AND meta are seen. Before meta, both the
    // semantic parse and the accounting walk must reject the file (they never disagree).
    let data = cat(&[
        ftyp(b"heic"),
        b"\x00\x00\x00\x40junk-oversized-box".to_vec(),
    ]);
    assert!(HeifContainer::parse(&data).is_err());
}

#[test]
fn c2pa_bearing_file_accounts_for_every_byte() {
    // The C2PA ContentProvenanceBox is an ordinary top-level `uuid` box, so the every-byte invariant
    // must hold over it unchanged — and the located store must sit inside that box's segment.
    let clean = clean_file(1, vec![hvc1_item(1, vec![7, 7, 7, 7])]);
    let store = jumbf_store(b"opaque-manifest-store");
    let provenance = c2pa_box("manifest", Some(0), &store, &[0xEE; 8]);
    let data = cat(&[clean.clone(), provenance.clone()]);
    let c = HeifContainer::parse(&data).unwrap();

    assert_covers(&c, data.len());
    assert_eq!(box_types(&c), vec![*b"ftyp", *b"meta", *b"mdat", *b"uuid"]);
    assert!(c.appended_stream().is_none());
    assert!(c.trailer().is_none());

    let uuid_segment = c.segments().last().unwrap();
    assert_eq!(uuid_segment.range, clean.len()..data.len());

    let found = c.c2pa().expect("manifest store located");
    assert_eq!(found.bytes, store.as_slice());
    // Strictly inside the box segment: the framing before it and the padding after it are still
    // claimed by that segment, so no byte is orphaned or double-counted.
    assert!(uuid_segment.range.start < found.range.start);
    assert!(found.range.end < uuid_segment.range.end);
    assert_eq!(&data[found.range.clone()], store.as_slice());
}
