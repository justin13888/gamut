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
        .encode_cfa(&raw, &profile, &mut dng)
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
