//! Authoritative conformance: every DNG gamut-dng writes must be accepted by the **Adobe DNG SDK**
//! (the `gamut-dng-oracle`, which runs the SDK's parse → build-negative → read-stage-1 flow).

mod common;

use gamut_dng::{ByteOrder, DngEncoder};

fn encode(order: ByteOrder, width: u32, height: u32, bits: u16) -> Vec<u8> {
    let raw = common::sample_raw(width, height, bits);
    let profile = common::sample_profile();
    let mut dng = Vec::new();
    DngEncoder::new()
        .with_byte_order(order)
        .encode(&raw, &profile, &mut dng)
        .expect("encode");
    dng
}

#[test]
fn adobe_sdk_validates_le_16bit_cfa() {
    let dng = encode(ByteOrder::LittleEndian, 64, 48, 16);
    gamut_dng_oracle::validate_dng(&dng)
        .expect("Adobe DNG SDK must accept gamut's little-endian DNG");
}

#[test]
fn adobe_sdk_validates_be_16bit_cfa() {
    let dng = encode(ByteOrder::BigEndian, 48, 32, 16);
    gamut_dng_oracle::validate_dng(&dng).expect("Adobe DNG SDK must accept gamut's big-endian DNG");
}

#[test]
fn adobe_sdk_validates_8bit_cfa() {
    let dng = encode(ByteOrder::LittleEndian, 32, 24, 8);
    gamut_dng_oracle::validate_dng(&dng).expect("Adobe DNG SDK must accept gamut's 8-bit DNG");
}

#[test]
fn adobe_sdk_validates_linear_raw() {
    let raw = common::sample_linear_raw(48, 36, 16);
    let mut dng = Vec::new();
    DngEncoder::new()
        .encode(&raw, &common::sample_profile(), &mut dng)
        .expect("encode");
    gamut_dng_oracle::validate_dng(&dng).expect("Adobe DNG SDK must accept gamut's LinearRaw DNG");
}

#[test]
fn adobe_sdk_validates_full_calibration_profile() {
    let raw = common::sample_raw(48, 32, 16);
    let mut dng = Vec::new();
    DngEncoder::new()
        .encode(&raw, &common::sample_profile_full(), &mut dng)
        .expect("encode");
    gamut_dng_oracle::validate_dng(&dng)
        .expect("Adobe DNG SDK must accept a dual-illuminant / forward-matrix profile");
}

/// The Adobe SDK must decode the stage-1 samples back to exactly what we packed — the definitive
/// check that gamut's bit-packing (12/14/16-bit, MSB-first, byte-aligned rows) matches DNG.
#[test]
fn adobe_decodes_packed_cfa_samples_exactly() {
    for bits in [12u16, 14, 16] {
        let raw = common::sample_raw(64, 48, bits);
        let mut dng = Vec::new();
        DngEncoder::new()
            .encode(&raw, &common::sample_profile(), &mut dng)
            .expect("encode");
        let decoded = gamut_dng_oracle::read_raw_dng(&dng).expect("Adobe reads raw");
        assert_eq!((decoded.width, decoded.height, decoded.planes), (64, 48, 1));
        assert_eq!(
            decoded.samples,
            raw.samples(),
            "Adobe stage-1 must match the {bits}-bit input mosaic"
        );
    }
}

#[test]
fn adobe_decodes_linear_raw_samples_exactly() {
    let raw = common::sample_linear_raw(48, 36, 16);
    let mut dng = Vec::new();
    DngEncoder::new()
        .encode(&raw, &common::sample_profile(), &mut dng)
        .expect("encode");
    let decoded = gamut_dng_oracle::read_raw_dng(&dng).expect("Adobe reads raw");
    assert_eq!((decoded.width, decoded.height, decoded.planes), (48, 36, 3));
    assert_eq!(
        decoded.samples,
        raw.samples(),
        "Adobe stage-1 LinearRaw must match input"
    );
}
