//! C2PA manifest-store carriage (C2PA 2.4 §A.3.6, §18.5.5): the store is typed on both sides of
//! the codec, its entry sits in the last IFD of the main chain with its value last in the file,
//! its bytes cross verbatim whatever the TIFF byte order, the encoder reports the two disjoint
//! exclusion ranges a signer hashes around, a reservation of the same size is byte-identical to
//! a store outside those ranges, and the Adobe DNG SDK accepts the result.

mod common;

use gamut_dng::{
    ByteOrder, C2paExclusions, DngDecoder, DngEncodeReport, DngEncoder, DngMetadata, Range, RawTag,
    Value,
};
use gamut_ifd::c2pa::C2PA_MANIFEST_STORE;
use gamut_ifd::{align_word, read_header};

/// `len` bytes that are neither a palindrome nor periodic at any small stride, so a byte-swapped,
/// shifted or truncated copy cannot equal the original.
fn store(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 37 + 11) as u8).collect()
}

fn with_store(order: ByteOrder, bytes: Vec<u8>) -> DngEncoder {
    DngEncoder::new()
        .with_byte_order(order)
        .with_metadata(DngMetadata {
            c2pa: Some(bytes),
            ..Default::default()
        })
}

fn encode(encoder: &DngEncoder) -> (Vec<u8>, DngEncodeReport) {
    let raw = common::sample_raw(32, 24, 16);
    let mut dng = Vec::new();
    let report = encoder
        .encode_with_report(&raw, &common::sample_profile(), &mut dng)
        .expect("encode");
    assert_eq!(report.len, dng.len());
    (dng, report)
}

fn slice(bytes: &[u8], range: Range) -> &[u8] {
    &bytes[range.start as usize..range.end() as usize]
}

/// The store is the file's last bytes, verbatim in both byte orders; the count field is the
/// 4-byte word inside IFD 0 holding the store's length in the file's byte order; the two ranges
/// are disjoint with the count field first; the decoder returns the same bytes and the same
/// ranges; and the Adobe DNG SDK accepts the file.
#[test]
fn store_is_written_last_verbatim_and_both_ranges_are_reported() {
    for order in [ByteOrder::LittleEndian, ByteOrder::BigEndian] {
        let bytes = store(100);
        let (dng, report) = encode(&with_store(order, bytes.clone()));
        let excl = report.c2pa.expect("a store was written");

        assert_eq!(
            excl.store.end(),
            dng.len() as u64,
            "{order:?}: the store is last"
        );
        assert_eq!(
            slice(&dng, excl.store),
            bytes.as_slice(),
            "{order:?}: verbatim"
        );
        assert_eq!(excl.count_field.len, 4);
        let count = order.u32(slice(&dng, excl.count_field).try_into().expect("4 bytes"));
        assert_eq!(
            u64::from(count),
            excl.store.len,
            "{order:?}: the count is the length"
        );
        // Inside IFD 0's body, which starts at the header's first-IFD offset.
        let (_, _, ifd0) = read_header(&dng).expect("header");
        assert!(excl.count_field.start > ifd0);
        assert!(
            excl.count_field.end() < excl.store.start,
            "disjoint, count field first"
        );

        let decoded = DngDecoder::new().decode(&dng).expect("decode");
        assert_eq!(decoded.metadata.c2pa, Some(bytes.clone()), "{order:?}");
        assert_eq!(decoded.c2pa_exclusions, Some(excl), "{order:?}");
        assert_eq!(
            decoded.metadata.blocks(),
            vec![gamut_dng::MetadataBlock::C2pa(&bytes)]
        );
        assert!(
            !decoded
                .ifd0_extra
                .iter()
                .any(|t| t.tag == C2PA_MANIFEST_STORE),
            "a typed store is not also an extra"
        );

        gamut_dng_oracle::validate_dng(&dng)
            .expect("Adobe DNG SDK must accept a DNG carrying a C2PA manifest store");
    }
}

/// A reservation is `len` zero bytes exactly where a store of `len` bytes goes: the two files
/// are byte-identical outside the store range, and their reports agree — so a signer can hash
/// the reserved file around the reported ranges and overwrite the reservation in place.
#[test]
fn a_reservation_is_zero_filled_and_otherwise_identical_to_a_store_of_its_size() {
    let bytes = store(64);
    let (with, report) = encode(&with_store(ByteOrder::LittleEndian, bytes.clone()));
    let (reserved, reserved_report) = encode(&DngEncoder::new().with_c2pa_reserved(bytes.len()));
    assert_eq!(reserved_report, report);
    let excl = report.c2pa.expect("ranges");
    assert_eq!(
        slice(&reserved, excl.store),
        vec![0u8; bytes.len()].as_slice()
    );
    assert_eq!(
        &reserved[..excl.store.start as usize],
        &with[..excl.store.start as usize]
    );
    assert_eq!(reserved.len(), with.len());

    // Filling the reservation in place yields the store-carrying file, byte for byte.
    let mut filled = reserved;
    filled[excl.store.start as usize..].copy_from_slice(&bytes);
    assert_eq!(filled, with);
    gamut_dng_oracle::validate_dng(&filled).expect("Adobe DNG SDK must accept the filled file");
}

/// §A.3.6's reason for "end of file": a store of a different size changes the count field and
/// nothing else before the store — every other offset in the file is untouched.
#[test]
fn a_store_of_a_different_size_moves_no_other_offset() {
    let (small, small_report) = encode(&with_store(ByteOrder::LittleEndian, store(40)));
    let (large, large_report) = encode(&with_store(ByteOrder::LittleEndian, store(4000)));
    let (s, l) = (
        small_report.c2pa.expect("ranges"),
        large_report.c2pa.expect("ranges"),
    );
    assert_eq!(s.store.start, l.store.start);
    assert_eq!(s.count_field, l.count_field);
    let cf = s.count_field;
    assert_eq!(&small[..cf.start as usize], &large[..cf.start as usize]);
    assert_eq!(
        &small[cf.end() as usize..s.store.start as usize],
        &large[cf.end() as usize..l.store.start as usize]
    );
    assert_ne!(
        slice(&small, cf),
        slice(&large, cf),
        "only the count differs"
    );
}

/// BigTIFF widens the count field to 8 bytes; the store is still found, decoded and accepted.
#[test]
fn bigtiff_count_field_is_eight_bytes_wide() {
    let bytes = store(48);
    let (dng, report) =
        encode(&with_store(ByteOrder::BigEndian, bytes.clone()).with_big_tiff(true));
    let excl = report.c2pa.expect("ranges");
    assert_eq!(excl.count_field.len, 8);
    let count =
        ByteOrder::BigEndian.u64(slice(&dng, excl.count_field).try_into().expect("8 bytes"));
    assert_eq!(count, excl.store.len);
    assert_eq!(excl.store.end(), dng.len() as u64);
    let decoded = DngDecoder::new().decode(&dng).expect("decode");
    assert_eq!(decoded.metadata.c2pa, Some(bytes));
    assert_eq!(decoded.c2pa_exclusions, Some(excl));
    gamut_dng_oracle::validate_dng(&dng)
        .expect("Adobe DNG SDK must accept a BigTIFF DNG carrying a C2PA manifest store");
}

/// Without a store or a reservation the report carries no ranges, and the file is what
/// `encode` writes.
#[test]
fn without_a_store_the_report_has_no_ranges() {
    let raw = common::sample_raw(32, 24, 16);
    let mut plain = Vec::new();
    let n = DngEncoder::new()
        .encode(&raw, &common::sample_profile(), &mut plain)
        .expect("encode");
    let (dng, report) = encode(&DngEncoder::new());
    assert_eq!(report.c2pa, None);
    assert_eq!(report.len, n);
    assert_eq!(dng, plain);
    let decoded = DngDecoder::new().decode(&dng).expect("decode");
    assert_eq!(decoded.metadata.c2pa, None);
    assert_eq!(decoded.c2pa_exclusions, None);
}

/// A store and a reservation together, and a store too short to be a JUMBF box, are typed
/// errors raised before any pixel work.
#[test]
fn a_conflicting_or_too_short_store_is_refused() {
    let raw = common::sample_raw(32, 24, 16);
    let both = with_store(ByteOrder::LittleEndian, store(16)).with_c2pa_reserved(16);
    let error = both
        .encode(&raw, &common::sample_profile(), &mut Vec::new())
        .expect_err("both");
    assert_eq!(
        error.static_message(),
        Some("DNG: supply either a C2PA manifest store or a reservation, not both")
    );
    for encoder in [
        with_store(ByteOrder::LittleEndian, store(7)),
        DngEncoder::new().with_c2pa_reserved(7),
    ] {
        let error = encoder
            .encode(&raw, &common::sample_profile(), &mut Vec::new())
            .expect_err("short");
        assert_eq!(
            error.static_message(),
            Some("DNG: a C2PA manifest store is at least a JUMBF box header (8 bytes)")
        );
    }
    // Exactly the minimum is accepted.
    assert!(
        DngEncoder::new()
            .with_c2pa_reserved(8)
            .encode(&raw, &common::sample_profile(), &mut Vec::new())
            .is_ok()
    );
}

/// A `C2PA` tag whose type is not `UNDEFINED` is not a manifest store (§A.3.6 fixes the type at
/// 7): it decodes as an unmodelled extra, with no store and no ranges.
#[test]
fn a_mistyped_c2pa_tag_is_an_extra_not_a_store() {
    let bytes = store(24);
    let (mut dng, report) = encode(&with_store(ByteOrder::LittleEndian, bytes.clone()));
    let excl = report.c2pa.expect("ranges");
    // The entry's type word is the 2 bytes before its count field: 7 (UNDEFINED) -> 1 (BYTE),
    // the same element size, so the value still decodes.
    let type_at = excl.count_field.start as usize - 2;
    assert_eq!(&dng[type_at..type_at + 2], &[7, 0]);
    dng[type_at..type_at + 2].copy_from_slice(&[1, 0]);

    let decoded = DngDecoder::new().decode(&dng).expect("decode");
    assert_eq!(decoded.metadata.c2pa, None);
    assert_eq!(decoded.c2pa_exclusions, None);
    assert!(decoded.ifd0_extra.contains(&RawTag {
        tag: C2PA_MANIFEST_STORE,
        value: Value::Byte(bytes),
    }));
}

/// Appends a directory holding only `entries` to the end of `dng` and links IFD 0's next-IFD
/// pointer to it — the "only entity within a new IFD following the existing one" form §A.3.6
/// allows for a single-main-IFD asset. Little-endian classic TIFF; each entry is
/// `(tag, type, count, value/offset word)`. Returns the new directory's offset.
fn append_trailing_ifd(dng: &mut Vec<u8>, entries: &[(u16, u16, u32, u32)]) -> u64 {
    let le = ByteOrder::LittleEndian;
    let (_, _, ifd0) = read_header(dng).expect("header");
    let n0 = usize::from(le.u16(dng[ifd0 as usize..ifd0 as usize + 2].try_into().expect("2")));
    let next_at = ifd0 as usize + 2 + n0 * 12;
    assert_eq!(
        &dng[next_at..next_at + 4],
        &[0, 0, 0, 0],
        "IFD 0 was the last directory"
    );

    let body = align_word(dng.len() as u64);
    dng.resize(body as usize, 0);
    dng.extend_from_slice(&le.pack_u16(entries.len() as u16));
    for &(tag, ty, count, word) in entries {
        dng.extend_from_slice(&le.pack_u16(tag));
        dng.extend_from_slice(&le.pack_u16(ty));
        dng.extend_from_slice(&le.pack_u32(count));
        dng.extend_from_slice(&le.pack_u32(word));
    }
    dng.extend_from_slice(&[0, 0, 0, 0]);
    dng[next_at..next_at + 4].copy_from_slice(&le.pack_u32(body as u32));
    body
}

/// A store carried the other lawful way — as the only entry of a trailing IFD — is found there,
/// with ranges inside that directory, and the file is still fully classified.
#[test]
fn a_store_in_a_trailing_ifd_is_found_there() {
    let bytes = store(50);
    let (mut dng, _) = encode(&DngEncoder::new());
    // Body: count (2) + one entry (12) + next (4) = 18 bytes; the store follows it.
    let plain_len = dng.len();
    let body = align_word(plain_len as u64);
    let store_at = body + 18;
    append_trailing_ifd(
        &mut dng,
        &[(C2PA_MANIFEST_STORE, 7, bytes.len() as u32, store_at as u32)],
    );
    dng.extend_from_slice(&bytes);

    let decoded = DngDecoder::new().decode(&dng).expect("decode");
    assert_eq!(decoded.metadata.c2pa, Some(bytes.clone()));
    assert_eq!(
        decoded.c2pa_exclusions,
        Some(C2paExclusions {
            store: Range {
                start: store_at,
                len: bytes.len() as u64
            },
            count_field: Range {
                start: body + 2 + 4,
                len: 4
            },
        })
    );
    let report = gamut_dng::deconstruct(&dng).expect("deconstruct");
    assert!(
        report.segments.is_fully_classified(),
        "the trailing directory and its store are claimed: {report:?}"
    );
}

/// The entry in IFD 0 of a file whose main chain continues past it breaks §A.3.6 ("the last
/// IFD of the main-IFD chain"): it is not the asset's store, and stays an IFD 0 extra.
#[test]
fn a_store_entry_before_the_last_main_ifd_is_not_the_store() {
    let bytes = store(24);
    let (mut dng, _) = encode(&with_store(ByteOrder::LittleEndian, bytes.clone()));
    // Software (305) ASCII "x\0", inline: a harmless trailing page.
    append_trailing_ifd(&mut dng, &[(305, 2, 2, u32::from_le_bytes(*b"x\0\0\0"))]);

    let decoded = DngDecoder::new().decode(&dng).expect("decode");
    assert_eq!(decoded.metadata.c2pa, None);
    assert_eq!(decoded.c2pa_exclusions, None);
    assert!(decoded.ifd0_extra.contains(&RawTag {
        tag: C2PA_MANIFEST_STORE,
        value: Value::Undefined(bytes),
    }));
}
