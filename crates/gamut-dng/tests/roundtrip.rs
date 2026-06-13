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
    let raw = common::sample_linear_raw(48, 36, 16);
    let mut dng = Vec::new();
    DngEncoder::new()
        .encode(&raw, &common::sample_profile(), &mut dng)
        .expect("encode");
    let decoded = DngDecoder::new().decode(&dng).expect("decode");
    assert_eq!(decoded.raw, raw);
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
fn decoder_rejects_garbage() {
    assert!(DngDecoder::new().decode(b"not a dng").is_err());
    assert!(DngDecoder::new().decode(&[]).is_err());
}
