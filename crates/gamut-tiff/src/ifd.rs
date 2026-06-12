//! Codec-specific interpretations of two IFD tag values: the photometric interpretation and the
//! prediction scheme.
//!
//! The structural IFD model — the byte-order header, [`gamut_ifd::FieldType`] / [`gamut_ifd::Value`],
//! the [`gamut_ifd::Ifd`] chain, and the read/write spine — lives in the shared
//! [`gamut_ifd`](https://crates.io/crates/gamut-ifd) crate (re-exported from this crate's root).
//! What stays here is *codec semantics*: how a [`PhotometricInterpretation`] maps samples to colour
//! and which [`Predictor`] is applied before compression. These are the meanings the encoder and
//! decoder attach to the `PhotometricInterpretation` (262) and `Predictor` (317) tags, not part of
//! the container structure.

/// How pixel samples map to colour, stored in the `PhotometricInterpretation` tag (262).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhotometricInterpretation {
    /// `0` — bilevel/grayscale where `0` is white and the maximum value is black.
    WhiteIsZero,
    /// `1` — bilevel/grayscale where `0` is black and the maximum value is white.
    BlackIsZero,
    /// `2` — full-colour RGB.
    Rgb,
    /// `3` — palette colour: sample values index a `ColorMap`.
    Palette,
    /// `4` — a transparency mask (an auxiliary bilevel image for another image).
    TransparencyMask,
    /// `5` — CMYK separated colour (TIFF 6.0 §16).
    Cmyk,
    /// `6` — YCbCr colour (TIFF 6.0 §21).
    YCbCr,
    /// `8` — CIE L\*a\*b\* colour (TIFF 6.0 §23).
    CieLab,
}

impl PhotometricInterpretation {
    /// Returns the interpretation for an on-disk tag value, or `None` if unrecognised.
    #[must_use]
    pub fn from_code(code: u32) -> Option<Self> {
        Some(match code {
            0 => PhotometricInterpretation::WhiteIsZero,
            1 => PhotometricInterpretation::BlackIsZero,
            2 => PhotometricInterpretation::Rgb,
            3 => PhotometricInterpretation::Palette,
            4 => PhotometricInterpretation::TransparencyMask,
            5 => PhotometricInterpretation::Cmyk,
            6 => PhotometricInterpretation::YCbCr,
            8 => PhotometricInterpretation::CieLab,
            _ => return None,
        })
    }

    /// Returns the on-disk tag value.
    #[must_use]
    pub fn code(self) -> u16 {
        match self {
            PhotometricInterpretation::WhiteIsZero => 0,
            PhotometricInterpretation::BlackIsZero => 1,
            PhotometricInterpretation::Rgb => 2,
            PhotometricInterpretation::Palette => 3,
            PhotometricInterpretation::TransparencyMask => 4,
            PhotometricInterpretation::Cmyk => 5,
            PhotometricInterpretation::YCbCr => 6,
            PhotometricInterpretation::CieLab => 8,
        }
    }
}

/// The prediction scheme applied before compression, stored in the `Predictor` tag (317,
/// TIFF 6.0 §14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Predictor {
    /// `1` — no prediction.
    #[default]
    None,
    /// `2` — horizontal differencing: each sample is stored as its difference from the sample to
    /// its left.
    HorizontalDifferencing,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn photometric_codes_round_trip() {
        // Every recognised PhotometricInterpretation code maps back to itself; note `7` is
        // unused (skipped between Cmyk=5/YCbCr=6 and CieLab=8) and unknown codes return None.
        for code in [0u32, 1, 2, 3, 4, 5, 6, 8] {
            let p = PhotometricInterpretation::from_code(code).expect("known photometric");
            assert_eq!(u32::from(p.code()), code);
        }
        assert_eq!(PhotometricInterpretation::from_code(7), None);
        assert_eq!(PhotometricInterpretation::from_code(9), None);
    }

    #[test]
    fn predictor_defaults_to_none() {
        assert_eq!(Predictor::default(), Predictor::None);
        assert_ne!(Predictor::None, Predictor::HorizontalDifferencing);
    }
}
