//! CICP (ITU-T H.273 / ISO/IEC 23091-2) code points shared by the AVIF `colr` (nclx) box and the
//! AV1 sequence header `color_config` (AV1 §5.5.2 / §6.4.2).
//!
//! Each enum is `#[repr(u16)]` with discriminants equal to the spec code points, so
//! [`MatrixCoefficients::code_point`] (and the siblings) is just the discriminant. M0 uses
//! `MatrixCoefficients::Identity`, `ColourPrimaries::Bt709`, `TransferCharacteristics::Srgb`, and
//! `ColorRange::Full`; the remaining named values are included for M2/M4 extension.
//!
//! Naming follows each identifier's source spec: the H.273 types keep the spec's British
//! spelling ([`ColourPrimaries`], as published), while AV1-derived names use the AV1 spec's
//! American spelling ([`ColorRange`], after `color_range`).

/// Matrix coefficients (CICP `MatrixCoefficients`). `Identity` (0) carries RGB directly with no
/// colour transform and requires 4:4:4 — the basis for lossless RGB AVIF.
///
/// `#[non_exhaustive]`: H.273 defines further code points, added here as milestones need them.
#[repr(u16)]
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MatrixCoefficients {
    /// Identity (RGB / "GBR"); no luma–chroma transform. Requires 4:4:4. (Code point 0.)
    Identity = 0,
    /// BT.709 — KR=0.2126, KB=0.0722. (Code point 1.)
    Bt709 = 1,
    /// Unspecified. (Code point 2.)
    Unspecified = 2,
    /// BT.601 / SMPTE 170M — KR=0.299, KB=0.114. (Code point 6; BT.470 System B,G is
    /// point 5, with identical coefficients.)
    Bt601 = 6,
    /// YCgCo. (Code point 8.)
    YCgCo = 8,
    /// BT.2020 non-constant luminance. (Code point 9.)
    Bt2020Ncl = 9,
}

impl MatrixCoefficients {
    /// The CICP code point.
    #[must_use]
    pub fn code_point(self) -> u16 {
        self as u16
    }

    /// The [`MatrixCoefficients`] for `code_point`, or `None` for any point not modeled here (a
    /// later minor release may turn a `None` into `Some`). The inverse of
    /// [`MatrixCoefficients::code_point`], for typing a value parsed from a `colr` box or AV1
    /// header.
    #[must_use]
    pub fn from_code_point(code_point: u16) -> Option<Self> {
        match code_point {
            0 => Some(MatrixCoefficients::Identity),
            1 => Some(MatrixCoefficients::Bt709),
            2 => Some(MatrixCoefficients::Unspecified),
            6 => Some(MatrixCoefficients::Bt601),
            8 => Some(MatrixCoefficients::YCgCo),
            9 => Some(MatrixCoefficients::Bt2020Ncl),
            _ => None,
        }
    }
}

/// Colour primaries (CICP `ColourPrimaries`).
///
/// `#[non_exhaustive]`: H.273 defines further code points, added here as milestones need them.
#[doc(alias = "ColorPrimaries")]
#[repr(u16)]
#[non_exhaustive]
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

    /// The [`ColourPrimaries`] for `code_point`, or `None` for any point not modeled here (a
    /// later minor release may turn a `None` into `Some`). The inverse of
    /// [`ColourPrimaries::code_point`].
    #[must_use]
    pub fn from_code_point(code_point: u16) -> Option<Self> {
        match code_point {
            1 => Some(ColourPrimaries::Bt709),
            2 => Some(ColourPrimaries::Unspecified),
            5 => Some(ColourPrimaries::Bt601Pal),
            6 => Some(ColourPrimaries::Smpte170m),
            9 => Some(ColourPrimaries::Bt2020),
            12 => Some(ColourPrimaries::DisplayP3),
            _ => None,
        }
    }
}

/// Transfer characteristics (CICP `TransferCharacteristics`).
///
/// `#[non_exhaustive]`: H.273 defines further code points, added here as milestones need them.
#[repr(u16)]
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransferCharacteristics {
    /// BT.709. (Code point 1.)
    Bt709 = 1,
    /// Unspecified. (Code point 2.)
    Unspecified = 2,
    /// Linear — the identity transfer, `V = Lc`. (Code point 8.)
    Linear = 8,
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

    /// The [`TransferCharacteristics`] for `code_point`, or `None` for any point not modeled here
    /// (a later minor release may turn a `None` into `Some`). The inverse of
    /// [`TransferCharacteristics::code_point`].
    #[must_use]
    pub fn from_code_point(code_point: u16) -> Option<Self> {
        match code_point {
            1 => Some(TransferCharacteristics::Bt709),
            2 => Some(TransferCharacteristics::Unspecified),
            8 => Some(TransferCharacteristics::Linear),
            13 => Some(TransferCharacteristics::Srgb),
            14 => Some(TransferCharacteristics::Bt2020_10),
            16 => Some(TransferCharacteristics::Pq),
            18 => Some(TransferCharacteristics::Hlg),
            _ => None,
        }
    }
}

/// Sample value range (CICP `VideoFullRangeFlag`; AV1 `color_range`).
///
/// Besides signalling the range in a `colr` / AV1 header, this is the range selector for the
/// RGB ↔ YCbCr conversions in [`crate::ycbcr`] — so one type carries the choice end to end.
///
/// Deliberately exhaustive (unlike the code-point enums): the flag is a spec-complete single bit,
/// so consumers can match it exhaustively forever.
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

    /// The [`ColorRange`] for `flag` (0 or 1), or `None` for any other value. The inverse of
    /// [`ColorRange::flag`].
    #[must_use]
    pub fn from_flag(flag: u8) -> Option<Self> {
        match flag {
            0 => Some(ColorRange::Limited),
            1 => Some(ColorRange::Full),
            _ => None,
        }
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
        assert_eq!(TransferCharacteristics::Linear.code_point(), 8);
        assert_eq!(TransferCharacteristics::Srgb.code_point(), 13);
        assert_eq!(TransferCharacteristics::Pq.code_point(), 16);
        assert_eq!(ColorRange::Full.flag(), 1);
        assert_eq!(ColorRange::Limited.flag(), 0);
    }

    #[test]
    fn code_point_inverses_round_trip() {
        use ColourPrimaries as Cp;
        use MatrixCoefficients as Mc;
        use TransferCharacteristics as Tc;
        for mc in [
            Mc::Identity,
            Mc::Bt709,
            Mc::Unspecified,
            Mc::Bt601,
            Mc::YCgCo,
            Mc::Bt2020Ncl,
        ] {
            assert_eq!(Mc::from_code_point(mc.code_point()), Some(mc));
        }
        for cp in [
            Cp::Bt709,
            Cp::Unspecified,
            Cp::Bt601Pal,
            Cp::Smpte170m,
            Cp::Bt2020,
            Cp::DisplayP3,
        ] {
            assert_eq!(Cp::from_code_point(cp.code_point()), Some(cp));
        }
        for tc in [
            Tc::Bt709,
            Tc::Unspecified,
            Tc::Linear,
            Tc::Srgb,
            Tc::Bt2020_10,
            Tc::Pq,
            Tc::Hlg,
        ] {
            assert_eq!(Tc::from_code_point(tc.code_point()), Some(tc));
        }
        for range in [ColorRange::Limited, ColorRange::Full] {
            assert_eq!(ColorRange::from_flag(range.flag()), Some(range));
        }
        // Unmodeled points map to None (3 = "reserved" / not modeled in every table).
        assert_eq!(Mc::from_code_point(3), None);
        assert_eq!(Cp::from_code_point(3), None);
        assert_eq!(Tc::from_code_point(3), None);
        assert_eq!(ColorRange::from_flag(2), None);
    }
}
