//! Strict deconstruct (issue #197): encode a DNG, then prove the deconstruct accounts every byte
//! of the IFD tree (IFD 0 preview, the raw sub-IFD, Exif) and flags nothing unexpected.

mod common;

use gamut_dng::{Anomaly, ByteOrder, DeconstructReport, DngDecoder, DngEncoder, deconstruct};

/// Asserts the structural walk classified the whole file with nothing unrecognised —
/// **zero tolerance**: word-alignment padding must come back as typed `Padding` segments,
/// not tolerated gaps (issue #263).
fn assert_clean(r: &DeconstructReport) {
    assert!(
        r.segments.is_fully_classified(),
        "not fully classified: {r:?}"
    );
    assert!(r.unknown_fields.is_empty(), "unknown fields: {r:?}");
    assert!(
        r.unknown_tags.is_empty(),
        "unknown tags: {:?}",
        r.unknown_tags
    );
    assert!(r.anomalies.is_empty(), "anomalies: {:?}", r.anomalies);
    assert!(r.is_fully_accounted(), "not fully accounted: {r:?}");
}

#[test]
fn encoded_cfa_dng_is_accounted() {
    for &order in &[ByteOrder::LittleEndian, ByteOrder::BigEndian] {
        for bits in [12u16, 16] {
            let raw = common::sample_raw(32, 24, bits);
            let mut dng = Vec::new();
            DngEncoder::new()
                .with_byte_order(order)
                .encode(&raw, &common::sample_profile(), &mut dng)
                .expect("encode");
            let report = deconstruct(&dng).expect("deconstruct");
            assert_clean(&report);
            // The raw sub-IFD's strips are the bulk of the file — the Data segments must
            // cover most of it.
            let data_bytes: u64 = report
                .segments
                .segments
                .iter()
                .filter(|s| matches!(s.kind, gamut_ifd::SpanKind::Data(_)))
                .map(|s| s.range.len)
                .sum();
            assert!(data_bytes * 2 > report.segments.file_len, "{report:?}");
        }
    }
}

#[test]
fn full_profile_dng_is_accounted() {
    // The full profile exercises the optional matrix / calibration tags the matrix check inspects.
    let raw = common::sample_raw(16, 16, 16);
    let mut dng = Vec::new();
    DngEncoder::new()
        .encode(&raw, &common::sample_profile_full(), &mut dng)
        .expect("encode");
    let report = deconstruct(&dng).expect("deconstruct");
    assert_clean(&report);
}

#[test]
fn linear_raw_dng_is_accounted() {
    let raw = common::sample_linear_raw(20, 12, 16);
    let mut dng = Vec::new();
    DngEncoder::new()
        .encode(&raw, &common::sample_profile(), &mut dng)
        .expect("encode");
    let report = deconstruct(&dng).expect("deconstruct");
    assert_clean(&report);
}

#[test]
fn decoder_deconstruct_returns_image_and_report() {
    let raw = common::sample_raw(16, 16, 16);
    let mut dng = Vec::new();
    DngEncoder::new()
        .encode(&raw, &common::sample_profile(), &mut dng)
        .expect("encode");
    let (decoded, report) = DngDecoder::new().deconstruct(&dng).expect("deconstruct");
    // The decoded raw matches a plain decode, and the report comes alongside it.
    assert_eq!(decoded.raw, raw);
    assert_clean(&report);
}

#[test]
fn unknown_private_tag_is_flagged() {
    // Inject a private tag into the encoded stream by editing IFD 0 through a re-parse/re-write is
    // awkward; instead, build a minimal raw IFD by hand and confirm a private tag on it is flagged
    // while its bytes stay accounted.
    use gamut_ifd::{TiffFile, Value, Variant, write};

    let mut ifd0 = gamut_ifd::Ifd::new();
    // A minimal IFD 0: declare DNGVersion plus a private (unknown) tag.
    ifd0.set(50706, Value::Byte(vec![1, 7, 0, 0])); // DNGVersion
    ifd0.set(0x9999, Value::Long(vec![42])); // private/unknown tag
    let bytes = write(&TiffFile {
        order: ByteOrder::LittleEndian,
        variant: Variant::Classic,
        ifds: vec![ifd0],
    })
    .expect("write");
    let report = deconstruct(&bytes).expect("deconstruct");
    assert!(
        report.unknown_tags.iter().any(|u| u.tag == 0x9999),
        "{report:?}"
    );
    assert!(!report.is_fully_accounted());
    // The unknown tag's value is inline, so byte classification is unaffected.
    assert!(report.segments.is_fully_classified());
}

#[test]
fn unknown_compression_code_is_flagged() {
    use gamut_ifd::{TiffFile, Value, Variant, write};

    let mut ifd0 = gamut_ifd::Ifd::new();
    ifd0.set(50706, Value::Byte(vec![1, 7, 0, 0])); // DNGVersion
    ifd0.set(259, Value::Short(vec![999])); // Compression: not a recognised DNG code
    let bytes = write(&TiffFile {
        order: ByteOrder::LittleEndian,
        variant: Variant::Classic,
        ifds: vec![ifd0],
    })
    .expect("write");
    let report = deconstruct(&bytes).expect("deconstruct");
    assert!(
        report.anomalies.iter().any(|a| matches!(
            a,
            Anomaly::UnknownCode { tag, code, .. } if *tag == 259 && *code == 999
        )),
        "{report:?}"
    );
}

#[test]
fn missing_strip_byte_counts_is_flagged() {
    use gamut_ifd::{TiffFile, Value, Variant, write};

    let mut ifd0 = gamut_ifd::Ifd::new();
    ifd0.set(50706, Value::Byte(vec![1, 7, 0, 0])); // DNGVersion
    ifd0.set(256, Value::Short(vec![4])); // ImageWidth: an image IFD...
    ifd0.set(273, Value::Long(vec![1000])); // ...with StripOffsets but no StripByteCounts
    let bytes = write(&TiffFile {
        order: ByteOrder::LittleEndian,
        variant: Variant::Classic,
        ifds: vec![ifd0],
    })
    .expect("write");
    let report = deconstruct(&bytes).expect("deconstruct");
    assert!(
        report.anomalies.iter().any(|a| matches!(
            a,
            Anomaly::Structure {
                severity: gamut_dng::Severity::Error,
                ..
            }
        )),
        "{report:?}"
    );
}

#[test]
fn image_ifd_without_pixel_data_warns() {
    use gamut_ifd::{TiffFile, Value, Variant, write};

    let mut ifd0 = gamut_ifd::Ifd::new();
    ifd0.set(50706, Value::Byte(vec![1, 7, 0, 0])); // DNGVersion
    ifd0.set(256, Value::Short(vec![4])); // ImageWidth, but neither strips nor tiles
    let bytes = write(&TiffFile {
        order: ByteOrder::LittleEndian,
        variant: Variant::Classic,
        ifds: vec![ifd0],
    })
    .expect("write");
    let report = deconstruct(&bytes).expect("deconstruct");
    assert!(
        report.anomalies.iter().any(|a| matches!(
            a,
            Anomaly::Structure {
                severity: gamut_dng::Severity::Warning,
                ..
            }
        )),
        "{report:?}"
    );
}

#[test]
fn cyclic_sub_ifd_is_flagged_not_hung() {
    // Root @8 whose SubIFDs pointer (330) targets a child @26 pointing back at itself.
    let data: &[u8] = &[
        b'I', b'I', 0x2a, 0x00, 0x08, 0x00, 0x00, 0x00, //
        0x01, 0x00, 0x4a, 0x01, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, 0x1a, 0x00, 0x00, 0x00, //
        0x00, 0x00, 0x00, 0x00, //
        0x01, 0x00, 0x4a, 0x01, 0x04, 0x00, 0x01, 0x00, 0x00, 0x00, 0x1a, 0x00, 0x00, 0x00, //
        0x00, 0x00, 0x00, 0x00,
    ];
    let report = deconstruct(data).expect("deconstruct");
    assert!(
        report.anomalies.iter().any(|a| matches!(
            a,
            Anomaly::Structure {
                severity: gamut_dng::Severity::Error,
                ..
            }
        )),
        "{report:?}"
    );
}

#[test]
fn unparseable_camera_profile_is_flagged() {
    use gamut_ifd::{TiffFile, Value, Variant, write};

    let mut ifd0 = gamut_ifd::Ifd::new();
    ifd0.set(50706, Value::Byte(vec![1, 7, 0, 0])); // DNGVersion
    // ExtraCameraProfiles pointing at bytes that are not a camera-profile stream.
    ifd0.set(50933, Value::Long(vec![4]));
    let bytes = write(&TiffFile {
        order: ByteOrder::LittleEndian,
        variant: Variant::Classic,
        ifds: vec![ifd0],
    })
    .expect("write");
    let report = deconstruct(&bytes).expect("deconstruct");
    assert!(
        report.anomalies.iter().any(|a| matches!(
            a,
            Anomaly::Structure {
                severity: gamut_dng::Severity::Error,
                ..
            }
        )),
        "{report:?}"
    );
}

#[test]
fn unknown_field_type_entry_is_reported_and_preserved() {
    // A hand-patched unknown field-type code (0xF0) in IFD 0: reported in `unknown_fields`,
    // while the byte-level classification stays clean (the record sits inside the body claim).
    use gamut_ifd::{TiffFile, Value, Variant, write};

    let mut ifd0 = gamut_ifd::Ifd::new();
    ifd0.set(50706, Value::Byte(vec![1, 7, 0, 0])); // DNGVersion
    ifd0.set(0x9999, Value::Long(vec![42]));
    let mut bytes = write(&TiffFile {
        order: ByteOrder::LittleEndian,
        variant: Variant::Classic,
        ifds: vec![ifd0],
    })
    .expect("write");
    // Entry records start at 10 (header 8 + count 2), sorted by tag — 0x9999 (39321) sorts
    // before DNGVersion (50706), so its type code sits at 10 + 2.
    bytes[12] = 0xF0;
    let report = deconstruct(&bytes).expect("deconstruct");
    assert_eq!(report.unknown_fields.len(), 1, "{report:?}");
    assert_eq!(report.unknown_fields[0].tag, 0x9999);
    assert_eq!(report.unknown_fields[0].type_code, 0xF0);
    assert!(report.segments.is_fully_classified(), "{report:?}");
    assert!(!report.is_fully_accounted());
}

#[test]
fn malformed_color_matrix_is_flagged() {
    use gamut_ifd::{TiffFile, Value, Variant, write};

    let mut ifd0 = gamut_ifd::Ifd::new();
    ifd0.set(50706, Value::Byte(vec![1, 7, 0, 0])); // DNGVersion
    // ColorMatrix1 (50721) must be nine rationals; give it three.
    ifd0.set(50721, Value::SRational(vec![(1, 1), (0, 1), (0, 1)]));
    let bytes = write(&TiffFile {
        order: ByteOrder::LittleEndian,
        variant: Variant::Classic,
        ifds: vec![ifd0],
    })
    .expect("write");
    let report = deconstruct(&bytes).expect("deconstruct");
    assert!(
        report.anomalies.iter().any(|a| matches!(
            a,
            Anomaly::UnparsableTag { tag, .. } if *tag == 50721
        )),
        "{report:?}"
    );
}
