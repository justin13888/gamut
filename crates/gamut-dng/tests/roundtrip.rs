//! End-to-end round-trips: gamut encode → gamut decode must reproduce the raw image exactly, and
//! gamut's decoder must agree with the Adobe SDK on the stage-1 samples.

mod common;

use gamut_dng::{ByteOrder, DngDecoder, DngEncoder, RawImage};

fn encode_cfa(order: ByteOrder, w: u32, h: u32, bits: u16) -> (Vec<u8>, RawImage) {
    let raw = common::sample_raw(w, h, bits);
    let mut dng = Vec::new();
    DngEncoder::new()
        .with_byte_order(order)
        .encode(&raw, &common::sample_profile(), &mut dng)
        .expect("encode");
    (dng, raw)
}

#[test]
fn cfa_roundtrips_through_gamut() {
    for &order in &[ByteOrder::LittleEndian, ByteOrder::BigEndian] {
        for bits in [8u16, 10, 12, 14, 16] {
            let (dng, raw) = encode_cfa(order, 32, 24, bits);
            let decoded = DngDecoder::new().decode(&dng).expect("decode");
            assert_eq!(
                decoded.raw, raw,
                "raw must round-trip ({bits}-bit, {order:?})"
            );
            assert_eq!(decoded.dng_version, [1, 4, 0, 0]);
            // The colour matrix round-trips within the RATIONAL storage precision.
            assert!((decoded.profile.color_matrix1()[0] - 0.6722).abs() < 1e-5);
            assert_eq!(decoded.profile.unique_camera_model(), "gamut TestCam");
        }
    }
}

#[test]
fn linear_raw_roundtrips_through_gamut() {
    // Cover sub-byte depths too: with 3 planes the packed-row width is `width * planes`. The *odd*
    // width 47 makes `width * planes * bits` not a whole number of bytes at 10/12/14-bit, so each
    // row is padded — a wrong samples-per-row then mis-packs the stream (an even width would pad
    // identically either way and hide the bug).
    for bits in [10u16, 12, 14, 16] {
        let raw = common::sample_linear_raw(47, 36, bits);
        let mut dng = Vec::new();
        DngEncoder::new()
            .encode(&raw, &common::sample_profile(), &mut dng)
            .expect("encode");
        let decoded = DngDecoder::new().decode(&dng).expect("decode");
        assert_eq!(decoded.raw, raw, "{bits}-bit linear must round-trip");
    }
}

#[test]
fn full_profile_roundtrips_optional_fields() {
    let raw = common::sample_raw(16, 16, 16);
    let mut dng = Vec::new();
    DngEncoder::new()
        .encode(&raw, &common::sample_profile_full(), &mut dng)
        .expect("encode");
    let decoded = DngDecoder::new().decode(&dng).expect("decode");
    let p = &decoded.profile;
    assert!(p.second_illuminant().is_some());
    assert!(p.forward_matrices().0.is_some());
    assert!(p.camera_calibration().0.is_some());
    assert!(
        p.analog_balance().is_some(),
        "AnalogBalance must survive decode"
    );
    assert_eq!(p.profile_name(), Some("gamut Standard"));
    assert!((p.baseline_exposure().unwrap() - 0.5).abs() < 1e-5);
}

#[test]
fn gamut_and_adobe_decoders_agree() {
    // gamut's decoder and the Adobe SDK must extract identical stage-1 samples from gamut's file.
    for bits in [12u16, 14, 16] {
        let (dng, raw) = encode_cfa(ByteOrder::LittleEndian, 64, 48, bits);
        let gamut = DngDecoder::new().decode(&dng).expect("gamut decode");
        let adobe = gamut_dng_oracle::read_raw_dng(&dng).expect("adobe decode");
        assert_eq!(
            gamut.raw.samples(),
            raw.samples(),
            "gamut decode matches input"
        );
        assert_eq!(adobe.samples, raw.samples(), "adobe decode matches input");
        assert_eq!(
            gamut.raw.samples(),
            adobe.samples.as_slice(),
            "gamut and adobe decoders agree ({bits}-bit)"
        );
    }
}

#[test]
fn tiled_roundtrips_through_gamut() {
    use gamut_dng::Compression;
    // 48x40 with 32x32 tiles: a 2x2 grid whose right/bottom tiles carry 16 padding columns and
    // 24 padding rows — the edge-crop path. Sub-byte depths exercise per-tile row alignment.
    for compression in [
        Compression::Uncompressed,
        Compression::Deflate,
        Compression::LosslessJpeg,
    ] {
        for bits in [8u16, 10, 12, 16] {
            // Deflate is limited to whole-byte depths (the SDK reader's constraint, enforced at
            // encode).
            if compression == Compression::Deflate && !matches!(bits, 8 | 16) {
                continue;
            }
            for raw in [
                common::sample_raw(48, 40, bits),
                common::sample_linear_raw(40, 24, bits),
            ] {
                let mut dng = Vec::new();
                DngEncoder::new()
                    .with_compression(compression)
                    .with_tiling(32, 32)
                    .encode(&raw, &common::sample_profile(), &mut dng)
                    .expect("encode");
                let decoded = DngDecoder::new().decode(&dng).expect("decode");
                assert_eq!(
                    decoded.raw, raw,
                    "{compression:?} {bits}-bit tiled must round-trip"
                );
            }
        }
    }
}

/// JPEG XL (Compression 52546) round-trips, stripped and tiled: gamut encode (lossless) → gamut
/// decode must be bit-exact, and the Adobe SDK (real libjxl) must both validate the file and
/// read identical pixels. JXL DNG data is full-range 16-bit (the reference SDK's decode
/// semantics), so the fixtures are 16-bit.
#[test]
fn jxl_roundtrips_and_validates() {
    use gamut_dng::Compression;
    for raw in [
        common::sample_raw(48, 40, 16),
        common::sample_linear_raw(40, 24, 16),
    ] {
        for tiled in [false, true] {
            let mut enc = DngEncoder::new()
                .with_compression(Compression::JpegXl)
                .with_dng_version([1, 7, 0, 0])
                .with_backward_version([1, 7, 0, 0]);
            if tiled {
                enc = enc.with_tiling(32, 32);
            }
            let mut dng = Vec::new();
            enc.encode(&raw, &common::sample_profile(), &mut dng)
                .expect("encode");
            let decoded = DngDecoder::new().decode(&dng).expect("decode");
            assert_eq!(
                decoded.raw, raw,
                "JXL (tiled={tiled}) must round-trip bit-exact"
            );
            gamut_dng_oracle::validate_dng(&dng)
                .unwrap_or_else(|e| panic!("Adobe must accept a JXL DNG (tiled={tiled}): {e}"));
            let adobe = gamut_dng_oracle::read_raw_dng(&dng).expect("adobe decode");
            assert_eq!(
                adobe.samples,
                raw.samples(),
                "Adobe stage-1 must match the JXL input (tiled={tiled})"
            );
        }
    }
}

/// Sub-16-bit input under JPEG XL is rejected with a typed error: the DNG ecosystem decodes JXL
/// at full 16-bit range, so an N-bit-code-value file would misrender against its own levels.
#[test]
fn jxl_rejects_sub_16bit_samples() {
    use gamut_dng::Compression;
    let raw = common::sample_raw(32, 32, 12);
    let err = DngEncoder::new()
        .with_compression(Compression::JpegXl)
        .encode(&raw, &common::sample_profile(), &mut Vec::new())
        .unwrap_err();
    assert!(
        matches!(err, gamut_core::Error::Unsupported(m) if m.contains("16-bit")),
        "expected a 16-bit requirement error, got {err:?}"
    );
}

/// Lossy JXL: the file must be structurally valid, gamut/Adobe must agree on the lossy pixels to
/// within one code (independent conforming decoders round the float reconstruction
/// independently), and the JXLDistance/JXLEffort tags record the configured parameters.
#[test]
fn lossy_jxl_validates_and_decoders_agree() {
    use gamut_dng::Compression;
    let raw = common::sample_linear_raw(64, 48, 16);
    let mut dng = Vec::new();
    DngEncoder::new()
        .with_compression(Compression::JpegXl)
        .with_jxl_distance(1.0)
        .with_jxl_effort(5)
        .with_dng_version([1, 7, 0, 0])
        .with_backward_version([1, 7, 0, 0])
        .with_tiling(32, 32)
        .encode(&raw, &common::sample_profile(), &mut dng)
        .expect("encode");
    gamut_dng_oracle::validate_dng(&dng).expect("Adobe must accept a lossy JXL DNG");
    let decoded = DngDecoder::new().decode(&dng).expect("gamut decode");
    let adobe = gamut_dng_oracle::read_raw_dng(&dng).expect("adobe decode");
    for (i, (&ours, &theirs)) in decoded.raw.samples().iter().zip(&adobe.samples).enumerate() {
        assert!(
            (i64::from(ours) - i64::from(theirs)).abs() <= 1,
            "lossy pixel {i}: gamut {ours} vs Adobe {theirs}"
        );
    }

    // The encode parameters are recorded in the raw IFD.
    let file = gamut_ifd::read(&dng).expect("parse");
    let raw_off = file.ifds[0].get_u64_vec(330).expect("SubIFDs")[0];
    let raw_ifd = gamut_ifd::read_ifd_at(&dng, raw_off, file.order, file.variant).expect("raw IFD");
    assert_eq!(
        raw_ifd.get(52553),
        Some(&gamut_ifd::Value::Float(vec![1.0])),
        "JXLDistance"
    );
    assert_eq!(raw_ifd.get_u32(52554), Some(5), "JXLEffort");
}

#[test]
fn tiled_bigtiff_roundtrips_and_validates() {
    let raw = common::sample_raw(48, 32, 12);
    let mut dng = Vec::new();
    DngEncoder::new()
        .with_big_tiff(true)
        .with_dng_version([1, 7, 0, 0])
        .with_backward_version([1, 7, 0, 0])
        .with_tiling(16, 16)
        .encode(&raw, &common::sample_profile(), &mut dng)
        .expect("encode");
    let decoded = DngDecoder::new()
        .decode(&dng)
        .expect("decode tiled BigTIFF");
    assert_eq!(decoded.raw, raw);
    gamut_dng_oracle::validate_dng(&dng).expect("Adobe must accept a tiled BigTIFF DNG");
}

#[test]
fn bigtiff_roundtrips_and_validates() {
    let raw = common::sample_raw(48, 32, 16);
    let mut dng = Vec::new();
    DngEncoder::new()
        .with_big_tiff(true)
        .with_dng_version([1, 7, 0, 0])
        .with_backward_version([1, 7, 0, 0])
        .encode(&raw, &common::sample_profile(), &mut dng)
        .expect("encode");
    assert_eq!(&dng[2..3], &[0x2b], "BigTIFF magic 43");
    // gamut decodes its own BigTIFF...
    let decoded = DngDecoder::new().decode(&dng).expect("decode BigTIFF");
    assert_eq!(decoded.raw, raw);
    // ...and the Adobe SDK accepts it.
    gamut_dng_oracle::validate_dng(&dng).expect("Adobe DNG SDK must accept a BigTIFF DNG");
}

#[test]
fn deflate_roundtrips_and_validates() {
    use gamut_dng::Compression;
    let cases = [
        common::sample_raw(64, 48, 16),
        common::sample_linear_raw(48, 36, 16),
    ];
    for raw in cases {
        let mut dng = Vec::new();
        DngEncoder::new()
            .with_compression(Compression::Deflate)
            .encode(&raw, &common::sample_profile(), &mut dng)
            .expect("encode");
        // gamut decodes its own Deflate output...
        let decoded = DngDecoder::new().decode(&dng).expect("decode Deflate");
        assert_eq!(decoded.raw, raw);
        // ...and the Adobe SDK both validates and decodes it to the same samples.
        gamut_dng_oracle::validate_dng(&dng).expect("Adobe DNG SDK must accept a Deflate DNG");
        let adobe = gamut_dng_oracle::read_raw_dng(&dng).expect("adobe decode");
        assert_eq!(adobe.samples, raw.samples());
    }
}

#[test]
fn lossless_jpeg_roundtrips_and_validates() {
    use gamut_dng::Compression;
    let cases = [
        common::sample_raw(64, 48, 16),
        common::sample_raw(33, 21, 12), // odd width, 12-bit
        common::sample_linear_raw(48, 36, 16),
    ];
    for raw in cases {
        let mut dng = Vec::new();
        DngEncoder::new()
            .with_compression(Compression::LosslessJpeg)
            .encode(&raw, &common::sample_profile(), &mut dng)
            .expect("encode");
        // gamut round-trips its own lossless JPEG...
        let decoded = DngDecoder::new()
            .decode(&dng)
            .expect("decode lossless JPEG");
        assert_eq!(decoded.raw, raw);
        // ...and the Adobe SDK validates and decodes it to the same samples.
        gamut_dng_oracle::validate_dng(&dng)
            .expect("Adobe DNG SDK must accept a lossless-JPEG DNG");
        let adobe = gamut_dng_oracle::read_raw_dng(&dng).expect("adobe decode");
        assert_eq!(
            adobe.samples,
            raw.samples(),
            "Adobe must decode gamut's lossless JPEG pixel-for-pixel"
        );
    }
}

#[test]
fn metadata_embeds_and_roundtrips() {
    use gamut_dng::{DngMetadata, ExifMetadata};
    let raw = common::sample_raw(32, 24, 16);
    let meta = DngMetadata {
        exif: ExifMetadata {
            exposure_time: Some((1, 250)),
            f_number: Some((28, 10)),
            iso_speed: Some(400),
            date_time_original: Some("2026:06:13 12:00:00".to_owned()),
            focal_length: Some((50, 1)),
        },
        xmp: Some(br#"<x:xmpmeta xmlns:x="adobe:ns:meta/"></x:xmpmeta>"#.to_vec()),
        iptc: Some(vec![0x1c, 0x02, 0x05, 0x00, 0x03, b'a', b'b', b'c']),
        icc: Some(vec![0u8; 16]),
    };
    let mut dng = Vec::new();
    DngEncoder::new()
        .with_metadata(meta.clone())
        .encode(&raw, &common::sample_profile(), &mut dng)
        .expect("encode");

    // The Adobe SDK accepts a DNG carrying an EXIF sub-IFD + XMP/IPTC/ICC blocks.
    gamut_dng_oracle::validate_dng(&dng).expect("Adobe DNG SDK must accept a metadata-rich DNG");

    // gamut reconstructs every block.
    let decoded = DngDecoder::new().decode(&dng).expect("decode");
    let got = &decoded.metadata;
    assert_eq!(got.exif.exposure_time, Some((1, 250)));
    assert_eq!(got.exif.f_number, Some((28, 10)));
    assert_eq!(got.exif.iso_speed, Some(400));
    assert_eq!(
        got.exif.date_time_original.as_deref(),
        Some("2026:06:13 12:00:00")
    );
    assert_eq!(got.exif.focal_length, Some((50, 1)));
    assert_eq!(got.xmp, meta.xmp);
    assert_eq!(got.iptc, meta.iptc);
    assert_eq!(got.icc, meta.icc);
    // The raw image still round-trips alongside the metadata.
    assert_eq!(decoded.raw, raw);
}

#[test]
fn level_family_roundtrips_and_validates() {
    use gamut_dng::RawLevels;
    // A 2x2 black pattern with four distinct RATIONAL-exact values (multiples of 1/65536),
    // asymmetric per-column/per-row deltas, masked areas, and an active area that is a proper
    // subset of the sensor (12x8 active window inside 16x12).
    let levels = RawLevels::new(1, (2, 2), vec![62.25, 63.0, 64.5, 65.75], vec![4095.0])
        .unwrap()
        .with_black_delta_h((0..12).map(|c| f64::from(c) * 0.25 - 1.0).collect())
        .with_black_delta_v((0..8).map(|r| 0.5 - f64::from(r) * 0.125).collect());
    let raw = common::sample_raw(16, 12, 12)
        .with_active_area([2, 3, 10, 15])
        // The default crop is relative to the active area and must fit inside it.
        .with_default_crop([0, 0], [12, 8])
        .with_levels(levels.clone())
        .unwrap()
        .with_masked_areas(vec![[0, 0, 2, 16], [2, 0, 10, 3]]);
    let mut dng = Vec::new();
    DngEncoder::new()
        .encode(&raw, &common::sample_profile(), &mut dng)
        .expect("encode");

    // gamut reconstructs the whole model bit-exactly (the fixture values sit on the 1/65536
    // storage grid, so the RATIONAL round-trip is exact).
    let decoded = DngDecoder::new().decode(&dng).expect("decode");
    assert_eq!(decoded.raw.levels(), &levels);
    assert_eq!(decoded.raw.masked_areas(), &[[0, 0, 2, 16], [2, 0, 10, 3]]);
    assert_eq!(decoded.raw, raw);

    // The Adobe SDK accepts the pattern + delta + masked-area write.
    gamut_dng_oracle::validate_dng(&dng)
        .expect("Adobe DNG SDK must accept the full black-level family");
}

#[test]
fn linearization_table_roundtrips_and_validates() {
    use gamut_dng::RawLevels;
    // A 12-bit CFA whose stored values pass through a square-law lookup table before black
    // subtraction. The table has 4096 entries (one per stored code).
    let table: Vec<u16> = (0..4096u32)
        .map(|v| ((v * v) >> 8).min(65535) as u16)
        .collect();
    let levels = RawLevels::uniform(1, 64.0, 65535.0)
        .unwrap()
        .with_linearization_table(table.clone());
    let raw = common::sample_raw(16, 12, 12)
        .with_levels(levels.clone())
        .unwrap();
    let mut dng = Vec::new();
    DngEncoder::new()
        .encode(&raw, &common::sample_profile(), &mut dng)
        .expect("encode");
    let decoded = DngDecoder::new().decode(&dng).expect("decode");
    assert_eq!(
        decoded.raw.levels().linearization_table(),
        Some(table.as_slice())
    );
    assert_eq!(decoded.raw, raw);
    gamut_dng_oracle::validate_dng(&dng)
        .expect("Adobe DNG SDK must accept a LinearizationTable DNG");
}

#[test]
fn per_plane_white_levels_roundtrip() {
    use gamut_dng::RawLevels;
    // LinearRaw with three distinct per-plane whites and per-plane blacks (repeat 1x1, so the
    // black pattern is one value per plane).
    let levels = RawLevels::new(
        3,
        (1, 1),
        vec![16.0, 32.0, 48.0],
        vec![4000.0, 4050.0, 4095.0],
    )
    .unwrap();
    let raw = common::sample_linear_raw(24, 18, 12)
        .with_levels(levels.clone())
        .unwrap();
    let mut dng = Vec::new();
    DngEncoder::new()
        .encode(&raw, &common::sample_profile(), &mut dng)
        .expect("encode");
    let decoded = DngDecoder::new().decode(&dng).expect("decode");
    assert_eq!(decoded.raw.levels(), &levels);
    gamut_dng_oracle::validate_dng(&dng).expect("Adobe DNG SDK must accept per-plane levels");
}

#[test]
fn opcode_lists_roundtrip_and_validate() {
    use gamut_dng::{Opcode, OpcodeList};
    // Vendor-private (unknown) opcode IDs with the optional flag: the SDK parses known IDs'
    // parameter payloads strictly, but wraps unknown optional opcodes and carries them through —
    // exactly the pass-through contract this container test is about.
    let mut list1 = OpcodeList::new();
    list1.push(Opcode {
        id: 0xC000_0001,
        spec_version: [1, 3, 0, 0],
        flags: Opcode::FLAG_OPTIONAL,
        parameters: vec![1, 2, 3, 4],
    });
    let mut list3 = OpcodeList::new();
    list3.push(Opcode {
        id: 0xC000_0002,
        spec_version: [1, 3, 0, 0],
        flags: Opcode::FLAG_OPTIONAL | Opcode::FLAG_PREVIEW_SKIP,
        parameters: vec![0xDE, 0xAD],
    });
    let raw = common::sample_raw(16, 12, 16)
        .with_opcode_list1(list1.clone())
        .with_opcode_list3(list3.clone());
    let mut dng = Vec::new();
    DngEncoder::new()
        .encode(&raw, &common::sample_profile(), &mut dng)
        .expect("encode");

    let decoded = DngDecoder::new().decode(&dng).expect("decode");
    assert_eq!(decoded.raw.opcode_list1(), &list1);
    assert!(decoded.raw.opcode_list2().is_empty());
    assert_eq!(decoded.raw.opcode_list3(), &list3);
    assert_eq!(decoded.raw, raw);

    // The Adobe SDK accepts the file (the opcodes are flagged optional, so an SDK that chose to
    // execute them may skip these synthetic payloads).
    gamut_dng_oracle::validate_dng(&dng).expect("Adobe DNG SDK must accept opcode lists");
}

#[test]
fn non_optional_opcodes_raise_the_backward_version() {
    use gamut_dng::{Opcode, OpcodeList};
    let mut list2 = OpcodeList::new();
    list2.push(Opcode {
        id: gamut_dng::opcode::opcode_id::WARP_RECTILINEAR_2,
        spec_version: [1, 6, 0, 0],
        flags: 0, // non-optional: a reader must execute it, so it needs DNG >= 1.6
        parameters: vec![],
    });
    let raw = common::sample_raw(16, 12, 16).with_opcode_list2(list2);
    let mut dng = Vec::new();
    DngEncoder::new()
        .with_dng_version([1, 6, 0, 0])
        .with_backward_version([1, 1, 0, 0]) // too low: must be raised to 1.6.0.0
        .encode(&raw, &common::sample_profile(), &mut dng)
        .expect("encode");
    let file = gamut_ifd::read(&dng).expect("parse");
    assert_eq!(
        file.ifds[0].get(gamut_dng::tags::DNG_BACKWARD_VERSION),
        Some(&gamut_ifd::Value::Byte(vec![1, 6, 0, 0])),
        "DNGBackwardVersion must be raised to the non-optional opcode's version"
    );
    // An optional opcode of the same version must NOT raise it.
    let mut optional = OpcodeList::new();
    optional.push(Opcode {
        id: gamut_dng::opcode::opcode_id::WARP_RECTILINEAR_2,
        spec_version: [1, 6, 0, 0],
        flags: Opcode::FLAG_OPTIONAL,
        parameters: vec![],
    });
    let raw = common::sample_raw(16, 12, 16).with_opcode_list2(optional);
    let mut dng = Vec::new();
    DngEncoder::new()
        .with_backward_version([1, 1, 0, 0])
        .encode(&raw, &common::sample_profile(), &mut dng)
        .expect("encode");
    let file = gamut_ifd::read(&dng).expect("parse");
    assert_eq!(
        file.ifds[0].get(gamut_dng::tags::DNG_BACKWARD_VERSION),
        Some(&gamut_ifd::Value::Byte(vec![1, 1, 0, 0]))
    );
}

#[test]
fn single_value_black_and_white_levels_broadcast_on_decode() {
    // Writers (including pre-pattern gamut-dng) may store BlackLevel/WhiteLevel with count 1
    // even when SamplesPerPixel > 1; the decoder broadcasts the value to every cell/plane.
    use gamut_ifd::{ByteOrder, TiffFile, Value, Variant, read, read_ifd_at, write};

    let raw = common::sample_linear_raw(8, 6, 16);
    let mut dng = Vec::new();
    DngEncoder::new()
        .encode(&raw, &common::sample_profile(), &mut dng)
        .expect("encode");

    // Rewrite the raw sub-IFD's levels as count-1 tags via the IFD layer.
    let file = read(&dng).expect("parse");
    let ifd0 = &file.ifds[0];
    let raw_off = ifd0
        .get_u32(gamut_dng::tags::SUB_IFDS)
        .expect("SubIFDs pointer");
    let mut raw_ifd = read_ifd_at(&dng, raw_off.into(), file.order, file.variant).expect("raw IFD");
    raw_ifd.set(gamut_dng::tags::BLACK_LEVEL, Value::Short(vec![7]));
    raw_ifd.set(gamut_dng::tags::WHITE_LEVEL, Value::Long(vec![60000]));
    raw_ifd.set(
        gamut_dng::tags::BLACK_LEVEL_REPEAT_DIM,
        Value::Short(vec![1, 1]),
    );
    // Re-emit a minimal single-IFD file holding just the raw image (strip data inline).
    let strip_off = raw_ifd
        .get_u32_vec(gamut_dng::tags::STRIP_OFFSETS)
        .expect("offsets")[0] as usize;
    let strip_len = raw_ifd
        .get_u32_vec(gamut_dng::tags::STRIP_BYTE_COUNTS)
        .expect("counts")[0] as usize;
    let strip = dng[strip_off..strip_off + strip_len].to_vec();
    let mut rebuilt_ifd = raw_ifd.clone();
    // Required IFD0-side tags for the decoder's profile/version path.
    for &tag in &[
        gamut_dng::tags::DNG_VERSION,
        gamut_dng::tags::UNIQUE_CAMERA_MODEL,
        gamut_dng::tags::COLOR_MATRIX1,
        gamut_dng::tags::CALIBRATION_ILLUMINANT1,
        gamut_dng::tags::AS_SHOT_NEUTRAL,
    ] {
        if let Some(v) = ifd0.get(tag) {
            rebuilt_ifd.set(tag, v.clone());
        }
    }
    // Place the strip immediately after the written IFD structure (two-pass: size, then emit).
    let single = |ifd: gamut_ifd::Ifd| TiffFile {
        order: ByteOrder::LittleEndian,
        variant: Variant::Classic,
        ifds: vec![ifd],
    };
    let mut rebuilt = write(&single(rebuilt_ifd.clone())).expect("write");
    let data_at = rebuilt.len() as u32;
    rebuilt_ifd.set(gamut_dng::tags::STRIP_OFFSETS, Value::Long(vec![data_at]));
    rebuilt_ifd.set(
        gamut_dng::tags::STRIP_BYTE_COUNTS,
        Value::Long(vec![strip.len() as u32]),
    );
    rebuilt = write(&single(rebuilt_ifd)).expect("rewrite");
    rebuilt.extend_from_slice(&strip);

    let decoded = DngDecoder::new().decode(&rebuilt).expect("decode");
    assert_eq!(decoded.raw.levels().black(), &[7.0, 7.0, 7.0]);
    assert_eq!(decoded.raw.levels().white(), &[60000.0, 60000.0, 60000.0]);
}

#[test]
fn decoder_rejects_garbage() {
    assert!(DngDecoder::new().decode(b"not a dng").is_err());
    assert!(DngDecoder::new().decode(&[]).is_err());
}
