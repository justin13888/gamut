#![cfg(feature = "serde")]

//! Serde contract tests for the feature-gated core enum surface.

use gamut_core::ColorModel;

#[test]
fn color_models_round_trip_by_variant_name() {
    let all = [
        ColorModel::Gray,
        ColorModel::GrayAlpha,
        ColorModel::Rgb,
        ColorModel::Rgba,
        ColorModel::Cmyk,
        ColorModel::Bilevel,
        ColorModel::Indexed,
    ];

    assert_eq!(
        serde_json::to_string(&ColorModel::Rgba).unwrap(),
        "\"Rgba\""
    );
    for value in all {
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(serde_json::from_str::<ColorModel>(&json).unwrap(), value);
    }
    assert!(serde_json::from_str::<ColorModel>("\"FutureModel\"").is_err());
}
