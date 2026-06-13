//! Enumerated DNG tag *values* — the codes stored in the `Compression`,
//! `PhotometricInterpretation`, `CalibrationIlluminant`, `CFALayout`, `Predictor`, `SampleFormat`,
//! `ProfileEmbedPolicy`, and `PreviewColorSpace` tags, plus the `NewSubFileType` bit values.
//!
//! Values come from the **DNG 1.7.1.0** specification and the Adobe DNG SDK's `dng_tag_values.h`.
//! Each enum mirrors the structural-codec pattern used elsewhere in the workspace: a `from_code`
//! that rejects unknown codes (`None`) and a `code` that returns the on-disk value.

/// The compression scheme of an image's data, stored in the `Compression` tag (259).
///
/// `gamut-dng` encodes and decodes [`Uncompressed`](Self::Uncompressed),
/// [`LosslessJpeg`](Self::LosslessJpeg), and [`Deflate`](Self::Deflate). [`LossyJpeg`](Self::LossyJpeg)
/// and [`JpegXl`](Self::JpegXl) are recognised for completeness but are out of the current
/// encode/decode scope (see `STATUS.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Compression {
    /// `1` — uncompressed sample data.
    #[default]
    Uncompressed,
    /// `7` — lossless JPEG (Huffman + predictor); the standard DNG raw compression.
    LosslessJpeg,
    /// `8` — Deflate/ZIP (RFC 1951), used for integer and floating-point data.
    Deflate,
    /// `34892` — DNG lossy JPEG (8-bit `LinearRaw`/mask only). Out of scope.
    LossyJpeg,
    /// `52546` — JPEG XL (DNG 1.7). Out of scope (deferred to a `gamut-jxl`-backed follow-up).
    JpegXl,
}

impl Compression {
    /// Returns the compression for an on-disk tag value, or `None` if unrecognised.
    #[must_use]
    pub fn from_code(code: u16) -> Option<Self> {
        Some(match code {
            1 => Compression::Uncompressed,
            7 => Compression::LosslessJpeg,
            8 => Compression::Deflate,
            34892 => Compression::LossyJpeg,
            52546 => Compression::JpegXl,
            _ => return None,
        })
    }

    /// Returns the on-disk tag value.
    #[must_use]
    pub fn code(self) -> u16 {
        match self {
            Compression::Uncompressed => 1,
            Compression::LosslessJpeg => 7,
            Compression::Deflate => 8,
            Compression::LossyJpeg => 34892,
            Compression::JpegXl => 52546,
        }
    }

    /// Whether `gamut-dng` can currently encode and decode this scheme.
    #[must_use]
    pub fn is_supported(self) -> bool {
        matches!(
            self,
            Compression::Uncompressed | Compression::LosslessJpeg | Compression::Deflate
        )
    }
}

/// How pixel samples map to colour / raw photometry, stored in `PhotometricInterpretation` (262).
///
/// Only the DNG-relevant interpretations are modelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhotometricInterpretation {
    /// `1` — grayscale where `0` is black (used by previews and masks).
    BlackIsZero,
    /// `2` — full-colour RGB (used by previews and as `LinearRaw`'s base for demosaiced RGB).
    Rgb,
    /// `4` — a transparency mask.
    TransparencyMask,
    /// `32803` — a colour-filter-array (mosaic) raw image.
    Cfa,
    /// `34892` — a demosaiced ("linear") raw image, one sample per plane per pixel.
    LinearRaw,
    /// `51177` — a depth map.
    Depth,
    /// `52527` — a photometric (per-channel) mask.
    PhotometricMask,
}

impl PhotometricInterpretation {
    /// Returns the interpretation for an on-disk tag value, or `None` if unrecognised.
    #[must_use]
    pub fn from_code(code: u16) -> Option<Self> {
        Some(match code {
            1 => PhotometricInterpretation::BlackIsZero,
            2 => PhotometricInterpretation::Rgb,
            4 => PhotometricInterpretation::TransparencyMask,
            32803 => PhotometricInterpretation::Cfa,
            34892 => PhotometricInterpretation::LinearRaw,
            51177 => PhotometricInterpretation::Depth,
            52527 => PhotometricInterpretation::PhotometricMask,
            _ => return None,
        })
    }

    /// Returns the on-disk tag value.
    #[must_use]
    pub fn code(self) -> u16 {
        match self {
            PhotometricInterpretation::BlackIsZero => 1,
            PhotometricInterpretation::Rgb => 2,
            PhotometricInterpretation::TransparencyMask => 4,
            PhotometricInterpretation::Cfa => 32803,
            PhotometricInterpretation::LinearRaw => 34892,
            PhotometricInterpretation::Depth => 51177,
            PhotometricInterpretation::PhotometricMask => 52527,
        }
    }
}

/// A calibration illuminant, stored in `CalibrationIlluminant1/2/3` (and equal to the EXIF
/// `LightSource` codes).
///
/// [`Other`](Self::Other) (`255`) signals that spectral data is supplied separately via the
/// `IlluminantData1/2/3` tags instead of a named illuminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalibrationIlluminant {
    /// `0` — unknown.
    Unknown,
    /// `1` — daylight.
    Daylight,
    /// `2` — fluorescent.
    Fluorescent,
    /// `3` — tungsten (incandescent).
    Tungsten,
    /// `4` — flash.
    Flash,
    /// `9` — fine weather.
    FineWeather,
    /// `10` — cloudy weather.
    CloudyWeather,
    /// `11` — shade.
    Shade,
    /// `12` — daylight fluorescent (D, 5700–7100 K).
    DaylightFluorescent,
    /// `13` — day-white fluorescent (N, 4600–5500 K).
    DayWhiteFluorescent,
    /// `14` — cool-white fluorescent (W, 3800–4500 K).
    CoolWhiteFluorescent,
    /// `15` — white fluorescent (WW, 3250–3800 K).
    WhiteFluorescent,
    /// `16` — warm-white fluorescent (L, 2600–3250 K).
    WarmWhiteFluorescent,
    /// `17` — Standard Light A.
    StandardLightA,
    /// `18` — Standard Light B.
    StandardLightB,
    /// `19` — Standard Light C.
    StandardLightC,
    /// `20` — D55.
    D55,
    /// `21` — D65.
    D65,
    /// `22` — D75.
    D75,
    /// `23` — D50.
    D50,
    /// `24` — ISO studio tungsten.
    IsoStudioTungsten,
    /// `255` — other (spectral data given via `IlluminantData`).
    Other,
}

impl CalibrationIlluminant {
    /// Returns the illuminant for an on-disk tag value, or `None` if unrecognised.
    #[must_use]
    pub fn from_code(code: u16) -> Option<Self> {
        Some(match code {
            0 => CalibrationIlluminant::Unknown,
            1 => CalibrationIlluminant::Daylight,
            2 => CalibrationIlluminant::Fluorescent,
            3 => CalibrationIlluminant::Tungsten,
            4 => CalibrationIlluminant::Flash,
            9 => CalibrationIlluminant::FineWeather,
            10 => CalibrationIlluminant::CloudyWeather,
            11 => CalibrationIlluminant::Shade,
            12 => CalibrationIlluminant::DaylightFluorescent,
            13 => CalibrationIlluminant::DayWhiteFluorescent,
            14 => CalibrationIlluminant::CoolWhiteFluorescent,
            15 => CalibrationIlluminant::WhiteFluorescent,
            16 => CalibrationIlluminant::WarmWhiteFluorescent,
            17 => CalibrationIlluminant::StandardLightA,
            18 => CalibrationIlluminant::StandardLightB,
            19 => CalibrationIlluminant::StandardLightC,
            20 => CalibrationIlluminant::D55,
            21 => CalibrationIlluminant::D65,
            22 => CalibrationIlluminant::D75,
            23 => CalibrationIlluminant::D50,
            24 => CalibrationIlluminant::IsoStudioTungsten,
            255 => CalibrationIlluminant::Other,
            _ => return None,
        })
    }

    /// Returns the on-disk tag value.
    #[must_use]
    pub fn code(self) -> u16 {
        match self {
            CalibrationIlluminant::Unknown => 0,
            CalibrationIlluminant::Daylight => 1,
            CalibrationIlluminant::Fluorescent => 2,
            CalibrationIlluminant::Tungsten => 3,
            CalibrationIlluminant::Flash => 4,
            CalibrationIlluminant::FineWeather => 9,
            CalibrationIlluminant::CloudyWeather => 10,
            CalibrationIlluminant::Shade => 11,
            CalibrationIlluminant::DaylightFluorescent => 12,
            CalibrationIlluminant::DayWhiteFluorescent => 13,
            CalibrationIlluminant::CoolWhiteFluorescent => 14,
            CalibrationIlluminant::WhiteFluorescent => 15,
            CalibrationIlluminant::WarmWhiteFluorescent => 16,
            CalibrationIlluminant::StandardLightA => 17,
            CalibrationIlluminant::StandardLightB => 18,
            CalibrationIlluminant::StandardLightC => 19,
            CalibrationIlluminant::D55 => 20,
            CalibrationIlluminant::D65 => 21,
            CalibrationIlluminant::D75 => 22,
            CalibrationIlluminant::D50 => 23,
            CalibrationIlluminant::IsoStudioTungsten => 24,
            CalibrationIlluminant::Other => 255,
        }
    }
}

/// The physical layout of the colour filter array, stored in `CFALayout` (50711).
///
/// `1` is the common rectangular (square) grid (e.g. ordinary Bayer); `2`–`9` are the staggered
/// layouts where alternate rows or columns are offset by half a pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CfaLayout {
    /// `1` — rectangular (square) layout.
    #[default]
    Rectangular,
    /// `2` — staggered layout A (even columns shifted down by 1/2 row).
    StaggeredA,
    /// `3` — staggered layout B (even columns shifted up by 1/2 row).
    StaggeredB,
    /// `4` — staggered layout C (even rows shifted right by 1/2 column).
    StaggeredC,
    /// `5` — staggered layout D (even rows shifted left by 1/2 column).
    StaggeredD,
    /// `6` — staggered layout E.
    StaggeredE,
    /// `7` — staggered layout F.
    StaggeredF,
    /// `8` — staggered layout G.
    StaggeredG,
    /// `9` — staggered layout H.
    StaggeredH,
}

impl CfaLayout {
    /// Returns the layout for an on-disk tag value, or `None` if out of range.
    #[must_use]
    pub fn from_code(code: u16) -> Option<Self> {
        Some(match code {
            1 => CfaLayout::Rectangular,
            2 => CfaLayout::StaggeredA,
            3 => CfaLayout::StaggeredB,
            4 => CfaLayout::StaggeredC,
            5 => CfaLayout::StaggeredD,
            6 => CfaLayout::StaggeredE,
            7 => CfaLayout::StaggeredF,
            8 => CfaLayout::StaggeredG,
            9 => CfaLayout::StaggeredH,
            _ => return None,
        })
    }

    /// Returns the on-disk tag value.
    #[must_use]
    pub fn code(self) -> u16 {
        match self {
            CfaLayout::Rectangular => 1,
            CfaLayout::StaggeredA => 2,
            CfaLayout::StaggeredB => 3,
            CfaLayout::StaggeredC => 4,
            CfaLayout::StaggeredD => 5,
            CfaLayout::StaggeredE => 6,
            CfaLayout::StaggeredF => 7,
            CfaLayout::StaggeredG => 8,
            CfaLayout::StaggeredH => 9,
        }
    }
}

/// The prediction scheme applied before compression, stored in the `Predictor` tag (317).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Predictor {
    /// `1` — no prediction.
    #[default]
    None,
    /// `2` — horizontal differencing (each sample minus the one to its left).
    HorizontalDifference,
    /// `3` — floating-point horizontal differencing.
    FloatingPoint,
    /// `34892` — horizontal differencing with a stride of 2 samples.
    HorizontalDifferenceX2,
    /// `34893` — horizontal differencing with a stride of 4 samples.
    HorizontalDifferenceX4,
    /// `34894` — floating-point differencing with a stride of 2 samples.
    FloatingPointX2,
    /// `34895` — floating-point differencing with a stride of 4 samples.
    FloatingPointX4,
}

impl Predictor {
    /// Returns the predictor for an on-disk tag value, or `None` if unrecognised.
    #[must_use]
    pub fn from_code(code: u16) -> Option<Self> {
        Some(match code {
            1 => Predictor::None,
            2 => Predictor::HorizontalDifference,
            3 => Predictor::FloatingPoint,
            34892 => Predictor::HorizontalDifferenceX2,
            34893 => Predictor::HorizontalDifferenceX4,
            34894 => Predictor::FloatingPointX2,
            34895 => Predictor::FloatingPointX4,
            _ => return None,
        })
    }

    /// Returns the on-disk tag value.
    #[must_use]
    pub fn code(self) -> u16 {
        match self {
            Predictor::None => 1,
            Predictor::HorizontalDifference => 2,
            Predictor::FloatingPoint => 3,
            Predictor::HorizontalDifferenceX2 => 34892,
            Predictor::HorizontalDifferenceX4 => 34893,
            Predictor::FloatingPointX2 => 34894,
            Predictor::FloatingPointX4 => 34895,
        }
    }
}

/// How each sample is encoded, stored in the `SampleFormat` tag (339).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SampleFormat {
    /// `1` — unsigned integer (the default for raw mosaic and linear-raw data).
    #[default]
    UnsignedInteger,
    /// `2` — signed integer.
    SignedInteger,
    /// `3` — IEEE floating point.
    FloatingPoint,
    /// `4` — undefined.
    Undefined,
}

impl SampleFormat {
    /// Returns the sample format for an on-disk tag value, or `None` if unrecognised.
    #[must_use]
    pub fn from_code(code: u16) -> Option<Self> {
        Some(match code {
            1 => SampleFormat::UnsignedInteger,
            2 => SampleFormat::SignedInteger,
            3 => SampleFormat::FloatingPoint,
            4 => SampleFormat::Undefined,
            _ => return None,
        })
    }

    /// Returns the on-disk tag value.
    #[must_use]
    pub fn code(self) -> u16 {
        match self {
            SampleFormat::UnsignedInteger => 1,
            SampleFormat::SignedInteger => 2,
            SampleFormat::FloatingPoint => 3,
            SampleFormat::Undefined => 4,
        }
    }
}

/// The embedding/usage policy of a camera profile, stored in `ProfileEmbedPolicy` (50941).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProfileEmbedPolicy {
    /// `0` — the profile may be freely copied.
    #[default]
    AllowCopying,
    /// `1` — embed the profile only in files it was used to render.
    EmbedIfUsed,
    /// `2` — never embed the profile.
    EmbedNever,
    /// `3` — no restrictions.
    NoRestrictions,
}

impl ProfileEmbedPolicy {
    /// Returns the policy for an on-disk tag value, or `None` if unrecognised.
    #[must_use]
    pub fn from_code(code: u32) -> Option<Self> {
        Some(match code {
            0 => ProfileEmbedPolicy::AllowCopying,
            1 => ProfileEmbedPolicy::EmbedIfUsed,
            2 => ProfileEmbedPolicy::EmbedNever,
            3 => ProfileEmbedPolicy::NoRestrictions,
            _ => return None,
        })
    }

    /// Returns the on-disk tag value.
    #[must_use]
    pub fn code(self) -> u32 {
        match self {
            ProfileEmbedPolicy::AllowCopying => 0,
            ProfileEmbedPolicy::EmbedIfUsed => 1,
            ProfileEmbedPolicy::EmbedNever => 2,
            ProfileEmbedPolicy::NoRestrictions => 3,
        }
    }
}

/// The colour space of a preview image, stored in `PreviewColorSpace` (50970).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PreviewColorSpace {
    /// `0` — unknown.
    #[default]
    Unknown,
    /// `1` — grayscale, gamma 2.2.
    GrayGamma22,
    /// `2` — sRGB.
    Srgb,
    /// `3` — Adobe RGB (1998).
    AdobeRgb,
    /// `4` — ProPhoto RGB.
    ProPhotoRgb,
}

impl PreviewColorSpace {
    /// Returns the colour space for an on-disk tag value, or `None` if unrecognised.
    #[must_use]
    pub fn from_code(code: u32) -> Option<Self> {
        Some(match code {
            0 => PreviewColorSpace::Unknown,
            1 => PreviewColorSpace::GrayGamma22,
            2 => PreviewColorSpace::Srgb,
            3 => PreviewColorSpace::AdobeRgb,
            4 => PreviewColorSpace::ProPhotoRgb,
            _ => return None,
        })
    }

    /// Returns the on-disk tag value.
    #[must_use]
    pub fn code(self) -> u32 {
        match self {
            PreviewColorSpace::Unknown => 0,
            PreviewColorSpace::GrayGamma22 => 1,
            PreviewColorSpace::Srgb => 2,
            PreviewColorSpace::AdobeRgb => 3,
            PreviewColorSpace::ProPhotoRgb => 4,
        }
    }
}

/// `NewSubFileType` (254) values DNG assigns to each subfile.
///
/// The TIFF field is a 32-bit bit field, but DNG uses a small set of specific values to label what
/// each IFD holds. The main raw image is `0`; a reduced-resolution preview is `1`.
pub mod new_subfile_type {
    /// The main (full-resolution) raw image.
    pub const MAIN_IMAGE: u32 = 0;
    /// A reduced-resolution preview or thumbnail.
    pub const PREVIEW_IMAGE: u32 = 1;
    /// A transparency mask.
    pub const TRANSPARENCY_MASK: u32 = 4;
    /// A depth map.
    pub const DEPTH_MAP: u32 = 8;
    /// An enhanced (e.g. denoised / super-resolution) version of the main image.
    pub const ENHANCED_IMAGE: u32 = 16;
    /// A gain map.
    pub const GAIN_MAP: u32 = 32;
    /// An alternate (non-primary) preview image.
    pub const ALT_PREVIEW_IMAGE: u32 = 0x0001_0001;
    /// A semantic mask.
    pub const SEMANTIC_MASK: u32 = 0x0001_0004;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compression_codes_round_trip() {
        for c in [
            Compression::Uncompressed,
            Compression::LosslessJpeg,
            Compression::Deflate,
            Compression::LossyJpeg,
            Compression::JpegXl,
        ] {
            assert_eq!(Compression::from_code(c.code()), Some(c));
        }
        assert_eq!(Compression::from_code(2), None); // CCITT etc. are not DNG raw schemes
        assert!(Compression::default().is_supported());
        assert!(Compression::LosslessJpeg.is_supported());
        assert!(!Compression::JpegXl.is_supported());
        assert!(!Compression::LossyJpeg.is_supported());
    }

    #[test]
    fn photometric_codes_round_trip() {
        for p in [
            PhotometricInterpretation::BlackIsZero,
            PhotometricInterpretation::Rgb,
            PhotometricInterpretation::TransparencyMask,
            PhotometricInterpretation::Cfa,
            PhotometricInterpretation::LinearRaw,
            PhotometricInterpretation::Depth,
            PhotometricInterpretation::PhotometricMask,
        ] {
            assert_eq!(PhotometricInterpretation::from_code(p.code()), Some(p));
        }
        assert_eq!(PhotometricInterpretation::from_code(3), None); // Palette is not a DNG photometry
    }

    #[test]
    fn illuminant_codes_round_trip() {
        // Spot-check the boundary and skipped codes plus the spectral-data sentinel.
        for code in [0u16, 1, 4, 9, 17, 21, 23, 24, 255] {
            let i = CalibrationIlluminant::from_code(code).expect("known illuminant");
            assert_eq!(i.code(), code);
        }
        assert_eq!(CalibrationIlluminant::from_code(5), None); // 5..=8 are unused
        assert_eq!(CalibrationIlluminant::D65.code(), 21);
        assert_eq!(CalibrationIlluminant::D50.code(), 23);
    }

    #[test]
    fn cfa_layout_codes_round_trip() {
        for code in 1..=9u16 {
            let l = CfaLayout::from_code(code).expect("known layout");
            assert_eq!(l.code(), code);
        }
        assert_eq!(CfaLayout::from_code(0), None);
        assert_eq!(CfaLayout::from_code(10), None);
        assert_eq!(CfaLayout::default(), CfaLayout::Rectangular);
    }

    #[test]
    fn predictor_codes_round_trip() {
        for p in [
            Predictor::None,
            Predictor::HorizontalDifference,
            Predictor::FloatingPoint,
            Predictor::HorizontalDifferenceX2,
            Predictor::HorizontalDifferenceX4,
            Predictor::FloatingPointX2,
            Predictor::FloatingPointX4,
        ] {
            assert_eq!(Predictor::from_code(p.code()), Some(p));
        }
        assert_eq!(Predictor::default(), Predictor::None);
        assert_eq!(Predictor::from_code(0), None);
    }

    #[test]
    fn other_enums_round_trip() {
        for s in [
            SampleFormat::UnsignedInteger,
            SampleFormat::SignedInteger,
            SampleFormat::FloatingPoint,
            SampleFormat::Undefined,
        ] {
            assert_eq!(SampleFormat::from_code(s.code()), Some(s));
        }
        assert_eq!(SampleFormat::default(), SampleFormat::UnsignedInteger);
        assert_eq!(SampleFormat::from_code(0), None);

        for p in [
            ProfileEmbedPolicy::AllowCopying,
            ProfileEmbedPolicy::EmbedIfUsed,
            ProfileEmbedPolicy::EmbedNever,
            ProfileEmbedPolicy::NoRestrictions,
        ] {
            assert_eq!(ProfileEmbedPolicy::from_code(p.code()), Some(p));
        }
        assert_eq!(ProfileEmbedPolicy::from_code(4), None);

        for c in [
            PreviewColorSpace::Unknown,
            PreviewColorSpace::GrayGamma22,
            PreviewColorSpace::Srgb,
            PreviewColorSpace::AdobeRgb,
            PreviewColorSpace::ProPhotoRgb,
        ] {
            assert_eq!(PreviewColorSpace::from_code(c.code()), Some(c));
        }
        assert_eq!(PreviewColorSpace::from_code(5), None);
    }
}
