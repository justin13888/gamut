//! CICP (ITU-T H.273 / ISO/IEC 23091-2) code points shared by the AVIF `colr` (nclx) box and the
//! AV1 sequence header `color_config` (AV1 §5.5.2 / §6.4.2).
//!
//! Each enum is `#[repr(u16)]` with discriminants equal to the spec code points, so
//! [`MatrixCoefficients::code_point`] (and the siblings) is just the discriminant. M0 uses
//! `MatrixCoefficients::Identity`, `ColourPrimaries::Bt709`, `TransferCharacteristics::Srgb`, and
//! `ColorRange::Full`; the remaining named values are included for M2/M4 extension.

/// Matrix coefficients (CICP `MatrixCoefficients`). `Identity` (0) carries RGB directly with no
/// colour transform and requires 4:4:4 — the basis for lossless RGB AVIF.
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatrixCoefficients {
    /// Identity (RGB / "GBR"); no luma–chroma transform. Requires 4:4:4. (Code point 0.)
    Identity = 0,
    /// BT.709 — KR=0.2126, KB=0.0722. (Code point 1.)
    Bt709 = 1,
    /// Unspecified. (Code point 2.)
    Unspecified = 2,
    /// BT.601 / BT.470 System B,G. (Code point 6.)
    Bt601 = 6,
    /// BT.2020 non-constant luminance. (Code point 9.)
    Bt2020Ncl = 9,
    /// YCgCo. (Code point 8.)
    YCgCo = 8,
}

impl MatrixCoefficients {
    /// The CICP code point.
    #[must_use]
    pub fn code_point(self) -> u16 {
        self as u16
    }
}

/// Colour primaries (CICP `ColourPrimaries`).
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColourPrimaries {
    /// BT.709 (also sRGB primaries). (Code point 1.)
    Bt709 = 1,
    /// Unspecified. (Code point 2.)
    Unspecified = 2,
    /// BT.601 625-line / BT.470 System B,G. (Code point 5.)
    Bt601Pal = 5,
    /// SMPTE 170M (BT.601 525-line). (Code point 6.)
    Smpte170m = 6,
    /// BT.2020 / BT.2100. (Code point 9.)
    Bt2020 = 9,
    /// SMPTE EG 432-1 (Display P3). (Code point 12.)
    DisplayP3 = 12,
}

impl ColourPrimaries {
    /// The CICP code point.
    #[must_use]
    pub fn code_point(self) -> u16 {
        self as u16
    }
}

/// Transfer characteristics (CICP `TransferCharacteristics`).
#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransferCharacteristics {
    /// BT.709. (Code point 1.)
    Bt709 = 1,
    /// Unspecified. (Code point 2.)
    Unspecified = 2,
    /// sRGB / IEC 61966-2-1. (Code point 13.)
    Srgb = 13,
    /// BT.2020 10-bit. (Code point 14.)
    Bt2020_10 = 14,
    /// SMPTE ST 2084 (PQ). (Code point 16.)
    Pq = 16,
    /// ARIB STD-B67 (HLG). (Code point 18.)
    Hlg = 18,
}

impl TransferCharacteristics {
    /// The CICP code point.
    #[must_use]
    pub fn code_point(self) -> u16 {
        self as u16
    }
}

/// Sample value range (CICP `VideoFullRangeFlag`; AV1 `color_range`).
///
/// Besides signalling the range in a `colr` / AV1 header, this is the range selector for the
/// RGB ↔ YCbCr conversions in [`crate::ycbcr`] — so one type carries the choice end to end.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorRange {
    /// Studio / limited range (e.g. luma 16–235 for 8-bit). (Flag 0.)
    Limited = 0,
    /// Full range (0–255 for 8-bit). (Flag 1.)
    Full = 1,
}

impl ColorRange {
    /// The `color_range` / `full_range_flag` value.
    #[must_use]
    pub fn flag(self) -> u8 {
        self as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_points_match_spec() {
        assert_eq!(MatrixCoefficients::Identity.code_point(), 0);
        assert_eq!(MatrixCoefficients::Bt709.code_point(), 1);
        assert_eq!(ColourPrimaries::Bt709.code_point(), 1);
        // A second, non-1 primaries value pins `self as u16` (a constant `1` would pass on Bt709
        // alone).
        assert_eq!(ColourPrimaries::Bt2020.code_point(), 9);
        assert_eq!(ColourPrimaries::DisplayP3.code_point(), 12);
        assert_eq!(TransferCharacteristics::Srgb.code_point(), 13);
        assert_eq!(TransferCharacteristics::Pq.code_point(), 16);
        assert_eq!(ColorRange::Full.flag(), 1);
        assert_eq!(ColorRange::Limited.flag(), 0);
    }
}
