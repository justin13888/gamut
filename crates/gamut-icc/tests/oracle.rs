//! Differential conformance tests for `gamut-icc` against the reference CMM (Little-CMS), via the
//! dev-only [`lcms2_oracle`] crate. Profiles are synthesized in memory by lcms2, so no binary
//! `.icc` fixtures are committed; gamut-icc decodes the same bytes and the decoded values are
//! asserted equal to what lcms2 reports.

use gamut_icc::{IccProfile, ProfileHeader, S15Fixed16, Signature, TagData};
use lcms2_oracle::tag;

/// The Rec.709/sRGB primaries and D65 white point, for synthesizing matrix/TRC profiles.
const D65: [f64; 2] = [0.3127, 0.3290];
const REC709_PRIMARIES: [[f64; 2]; 3] = [[0.64, 0.33], [0.30, 0.60], [0.15, 0.06]];

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

/// gamut-icc finds exactly the tags the reference CMM reports, and decodes each (currently as `Raw`
/// with its type signature matching the element's leading four bytes).
#[test]
fn tag_set_matches_lcms() {
    let profiles = [
        ("srgb", lcms2_oracle::srgb()),
        ("gray", lcms2_oracle::gray([0.3127, 0.3290], 2.2)),
        ("lab4", lcms2_oracle::lab4()),
    ];
    for (label, profile) in profiles {
        let bytes = profile.to_bytes();
        let parsed = IccProfile::parse(&bytes).unwrap_or_else(|e| panic!("{label}: {e:?}"));

        let mut got: Vec<u32> = parsed.tags.iter().map(|(s, _)| s.to_u32()).collect();
        let mut want: Vec<u32> = (0..profile.tag_count())
            .map(|i| profile.tag_signature(i))
            .collect();
        got.sort_unstable();
        want.sort_unstable();
        assert_eq!(got, want, "{label}: tag signature set");

        for (_, data) in &parsed.tags {
            if let TagData::Raw { type_sig, bytes } = data {
                assert_eq!(&type_sig.0, &bytes[0..4], "{label}: raw type signature");
            }
        }
    }
}

/// The XYZ colorant/white-point tags decode to the same tristimulus values lcms reads back.
#[test]
fn xyz_tags_match_lcms() {
    let profile = lcms2_oracle::srgb();
    let parsed = IccProfile::parse(&profile.to_bytes()).unwrap();
    for tagsig in [
        tag::MEDIA_WHITE_POINT,
        tag::RED_COLORANT,
        tag::GREEN_COLORANT,
        tag::BLUE_COLORANT,
    ] {
        let data = parsed
            .get(Signature::from_u32(tagsig))
            .expect("tag present");
        let TagData::Xyz(values) = data else {
            panic!("expected XYZ for {tagsig:#010x}");
        };
        let ours = values[0].to_f64();
        let theirs = profile.read_xyz(tagsig).expect("lcms reads XYZ");
        for k in 0..3 {
            assert!(
                (ours[k] - theirs[k]).abs() < 2.0 / 65536.0,
                "{tagsig:#010x}[{k}]: {} vs {}",
                ours[k],
                theirs[k]
            );
        }
    }
}

/// Decoded tone curves evaluate to the same values as lcms, for both a parametric (sRGB) and a
/// pure-gamma TRC. Sampling `eval` at points across [0, 1] pins the curve formulas.
#[test]
fn tone_curves_match_lcms() {
    let cases = [
        ("srgb", lcms2_oracle::srgb()),
        (
            "gamma2.2",
            lcms2_oracle::rgb_matrix_shaper(D65, REC709_PRIMARIES, [2.2, 2.2, 2.2]),
        ),
    ];
    for (label, profile) in cases {
        let parsed = IccProfile::parse(&profile.to_bytes()).unwrap();
        let data = parsed
            .get(Signature::from_u32(tag::RED_TRC))
            .expect("rTRC present");
        for i in 0..=16 {
            let x = f64::from(i) / 16.0;
            let ours = eval_curve(data, x);
            let theirs = f64::from(
                profile
                    .eval_tone_curve(tag::RED_TRC, x as f32)
                    .expect("lcms evaluates the curve"),
            );
            assert!(
                (ours - theirs).abs() < 2.0e-3,
                "{label} @ {x}: {ours} vs {theirs}"
            );
        }
    }
}

/// A non-D50 (here D65) matrix profile carries a chromatic-adaptation matrix, stored as an
/// `s15Fixed16ArrayType` of nine values (the 3×3 matrix).
#[test]
fn chad_decodes_as_s15fixed16_array() {
    let profile = lcms2_oracle::rgb_matrix_shaper(D65, REC709_PRIMARIES, [2.2, 2.2, 2.2]);
    let parsed = IccProfile::parse(&profile.to_bytes()).unwrap();
    let data = parsed
        .get(Signature::from_u32(tag::CHROMATIC_ADAPTATION))
        .expect("chad present");
    let TagData::S15Fixed16Array(values) = data else {
        panic!("expected sf32 chad");
    };
    assert_eq!(values.len(), 9);
}

/// The profile description decodes to the same text lcms reads back, in both the v4
/// (`multiLocalizedUnicode`) and v2 (`textDescription`) representations.
#[test]
fn descriptions_match_lcms() {
    // v4: 'desc' is a multiLocalizedUnicode element.
    let profile = lcms2_oracle::srgb();
    let parsed = IccProfile::parse(&profile.to_bytes()).unwrap();
    let ours = match parsed
        .get(Signature::from_u32(tag::PROFILE_DESCRIPTION))
        .expect("desc present")
    {
        TagData::MultiLocalizedUnicode(mluc) => mluc
            .text(b"en", b"US")
            .or_else(|| mluc.first())
            .expect("a description record")
            .to_owned(),
        other => panic!("expected mluc, got {other:?}"),
    };
    let theirs = profile
        .read_mlu_ascii(tag::PROFILE_DESCRIPTION, b"en", b"US")
        .expect("lcms reads desc");
    assert_eq!(ours, theirs);

    // v2: the same description is stored as a textDescription element.
    let profile = lcms2_oracle::srgb();
    profile.set_version(2.1);
    let parsed = IccProfile::parse(&profile.to_bytes()).unwrap();
    let ours = match parsed
        .get(Signature::from_u32(tag::PROFILE_DESCRIPTION))
        .expect("desc present")
    {
        TagData::TextDescription(desc) => desc.ascii.clone(),
        other => panic!("expected textDescription, got {other:?}"),
    };
    let theirs = profile
        .read_mlu_ascii(tag::PROFILE_DESCRIPTION, b"en", b"US")
        .expect("lcms reads desc");
    assert_eq!(ours, theirs);
}

/// Evaluates whichever tone-curve element a tag holds.
fn eval_curve(data: &TagData, x: f64) -> f64 {
    match data {
        TagData::Curve(curve) => curve.eval(x),
        TagData::ParametricCurve(curve) => curve.eval(x),
        other => panic!("expected a tone curve, got {other:?}"),
    }
}
