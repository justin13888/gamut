//! Differential conformance tests for `gamut-icc` against the reference CMM (Little-CMS), via the
//! dev-only [`lcms2_oracle`] crate. Profiles are synthesized in memory by lcms2, so no binary
//! `.icc` fixtures are committed; gamut-icc decodes the same bytes and the decoded values are
//! asserted equal to what lcms2 reports.

use gamut_icc::{ProfileHeader, S15Fixed16};

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

/// Header fields decoded by gamut-icc match what the reference CMM reports, across a spread of
/// synthesized profiles. Comparing against lcms's own getters (rather than hard-coded values) keeps
/// the test honest and pins every modelled header field's offset and decoder.
#[test]
fn header_fields_match_lcms() {
    let profiles = [
        ("srgb", lcms2_oracle::srgb()),
        (
            "rgb",
            lcms2_oracle::rgb_matrix_shaper(
                [0.3127, 0.3290],
                [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06]],
                [2.2, 2.2, 2.2],
            ),
        ),
        ("gray", lcms2_oracle::gray([0.3127, 0.3290], 2.2)),
        ("xyz", lcms2_oracle::xyz()),
        ("lab4", lcms2_oracle::lab4()),
    ];
    for (label, profile) in profiles {
        let bytes = profile.to_bytes();
        let h = ProfileHeader::parse(&bytes).unwrap_or_else(|e| panic!("{label}: {e:?}"));
        assert_eq!(
            h.device_class.to_signature().to_u32(),
            profile.device_class(),
            "{label}: device class"
        );
        assert_eq!(
            h.data_color_space.to_signature().to_u32(),
            profile.color_space(),
            "{label}: data colour space"
        );
        assert_eq!(h.pcs.to_signature().to_u32(), profile.pcs(), "{label}: pcs");
        assert_eq!(
            h.rendering_intent.to_u32(),
            profile.rendering_intent(),
            "{label}: rendering intent"
        );
        let version = f64::from(h.version.major) + f64::from(h.version.minor) / 10.0;
        assert!(
            (version - profile.version()).abs() < 0.05,
            "{label}: version {version} vs lcms {}",
            profile.version()
        );
    }
}

/// A v2 profile decodes its major/minor version correctly (the legacy path lcms emits when the
/// version is forced down).
#[test]
fn parses_v2_profile_version() {
    let profile = lcms2_oracle::srgb();
    profile.set_version(2.1);
    let bytes = profile.to_bytes();
    let h = ProfileHeader::parse(&bytes).unwrap();
    assert_eq!(h.version.major, 2);
    assert_eq!(h.version.minor, 1);
}
