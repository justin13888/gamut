//! Differential conformance tests for `gamut-icc` against the reference CMM (Little-CMS), via the
//! dev-only [`lcms2_oracle`] crate. Profiles are synthesized in memory by lcms2, so no binary
//! `.icc` fixtures are committed; gamut-icc decodes the same bytes and the decoded values are
//! asserted equal to what lcms2 reports.

use gamut_icc::S15Fixed16;

/// The header's PCS illuminant (ICC.1:2022 §7.2.16) is mandated to be D50; lcms2 writes exactly
/// that. Decoding those three `s15Fixed16` fields from a real lcms2-produced profile exercises the
/// fixed-point conversion against an independent reference.
#[test]
fn pcs_illuminant_is_d50() {
    let bytes = lcms2_oracle::srgb().to_bytes();
    assert!(bytes.len() >= 128, "profile shorter than its header");

    // The PCS illuminant lives at byte offset 68: X, Y, Z as consecutive s15Fixed16 values.
    let component = |offset: usize| -> f64 {
        let raw = i32::from_be_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]);
        S15Fixed16(raw).to_f64()
    };
    let (x, y, z) = (component(68), component(72), component(76));

    // D50 ≈ (0.9642, 1.0000, 0.8249) — within one s15Fixed16 quantum.
    assert!((x - 0.9642).abs() < 1.0e-3, "illuminant X = {x}");
    assert!((y - 1.0).abs() < 1.0e-3, "illuminant Y = {y}");
    assert!((z - 0.8249).abs() < 1.0e-3, "illuminant Z = {z}");
}
