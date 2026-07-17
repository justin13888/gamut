//! Authoritative conformance: every DNG gamut-dng writes must be accepted by the **Adobe DNG SDK**
//! (the `gamut-dng-oracle`, which runs the SDK's parse → build-negative → read-stage-1 flow), and
//! `RawImage::to_linear` must reproduce the SDK's stage-2 (chapter-5) linearization.

mod common;

use gamut_dng::{ByteOrder, DngDecoder, DngEncoder, RawImage, RawLevels};

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

/// Encodes `raw`, re-decodes it, and requires gamut's `to_linear` to match the Adobe SDK's
/// stage-2 image within ±1 of the 16-bit encoding (`round(linear * 65535)`) per sample.
///
/// The tolerance covers only rounding at `.5` boundaries (both sides compute in doubles); any
/// anchoring, phase, delta, or scale bug shifts values by whole black-level units — orders of
/// magnitude above one 16-bit step — so this is a genuine differential gate on the chapter-5
/// pipeline, exercised through the full encode → decode → linearize path.
fn assert_to_linear_matches_adobe(raw: &RawImage, what: &str) {
    let mut dng = Vec::new();
    DngEncoder::new()
        .encode(raw, &common::sample_profile(), &mut dng)
        .expect("encode");
    let decoded = DngDecoder::new().decode(&dng).expect("gamut decode");
    let gamut = decoded.raw.to_linear().expect("gamut to_linear");
    let adobe = gamut_dng_oracle::read_linear_dng(&dng).expect("Adobe stage-2");
    assert_eq!(
        (adobe.width, adobe.height, adobe.planes),
        (
            gamut.width,
            gamut.height,
            u32::from(gamut.samples_per_pixel)
        ),
        "{what}: stage-2 geometry must agree (active-area crop)"
    );
    assert_eq!(gamut.samples.len(), adobe.samples.len());
    for (i, (&ours, &theirs)) in gamut.samples.iter().zip(&adobe.samples).enumerate() {
        let ours16 = (f64::from(ours) * 65535.0).round() as i32;
        let diff = (ours16 - i32::from(theirs)).abs();
        assert!(
            diff <= 1,
            "{what}: sample {i} diverges — gamut {ours16} vs Adobe {theirs}"
        );
    }
}

#[test]
fn to_linear_matches_adobe_for_uniform_black() {
    let raw = common::sample_raw(32, 24, 12)
        .with_black_level(64.0)
        .expect("black");
    assert_to_linear_matches_adobe(&raw, "uniform black");
}

/// The anchoring gate: a 2x2 fractional black pattern under a non-full active area whose origin
/// has odd row/column parity, so anchoring the pattern at the image origin (instead of the
/// active-area origin) swaps every phase.
#[test]
fn to_linear_matches_adobe_for_black_pattern_with_active_area() {
    let levels =
        RawLevels::new(1, (2, 2), vec![62.25, 63.0, 64.5, 65.75], vec![4095.0]).expect("levels");
    let raw = common::sample_raw(16, 12, 12)
        .with_active_area([3, 5, 11, 13])
        .with_default_crop([0, 0], [8, 8])
        .with_levels(levels)
        .expect("levels");
    assert_to_linear_matches_adobe(&raw, "2x2 pattern + active area");
}

#[test]
fn to_linear_matches_adobe_for_black_deltas() {
    let levels = RawLevels::uniform(1, 64.0, 4095.0)
        .expect("levels")
        .with_black_delta_h((0..16).map(|c| f64::from(c) * 0.25 - 2.0).collect())
        .with_black_delta_v((0..12).map(|r| 1.5 - f64::from(r) * 0.5).collect());
    let raw = common::sample_raw(16, 12, 12)
        .with_levels(levels)
        .expect("levels");
    assert_to_linear_matches_adobe(&raw, "delta H+V");
}

#[test]
fn to_linear_matches_adobe_for_linearization_table() {
    // A square-law table over the full 12-bit input domain.
    let table: Vec<u16> = (0..4096u32)
        .map(|v| ((v * v) >> 8).min(65535) as u16)
        .collect();
    let levels = RawLevels::uniform(1, 100.0, 65535.0)
        .expect("levels")
        .with_linearization_table(table);
    let raw = common::sample_raw(24, 16, 12)
        .with_levels(levels)
        .expect("levels");
    assert_to_linear_matches_adobe(&raw, "linearization table");
}

#[test]
fn to_linear_matches_adobe_for_per_plane_whites() {
    let levels = RawLevels::new(
        3,
        (1, 1),
        vec![16.0, 32.0, 48.0],
        vec![4000.0, 4050.0, 4095.0],
    )
    .expect("levels");
    let raw = common::sample_linear_raw(24, 18, 12)
        .with_levels(levels)
        .expect("levels");
    assert_to_linear_matches_adobe(&raw, "per-plane whites");
}
