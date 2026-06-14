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
    // Cover sub-byte depths too: with 3 planes the packed-row width is `width * planes`, so a wrong
    // samples-per-row would mis-pack the (bit-packed) sub-byte cases.
    for bits in [10u16, 12, 14, 16] {
        let raw = common::sample_linear_raw(48, 36, bits);
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
fn decoder_rejects_garbage() {
    assert!(DngDecoder::new().decode(b"not a dng").is_err());
    assert!(DngDecoder::new().decode(&[]).is_err());
}
