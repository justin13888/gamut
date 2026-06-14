//! Source-profile bundle: ties a colour-science [`Gamut`] together with its
//! encoder-exact transfer and projects them back onto gamut's independent CICP
//! axes.
//!
//! gamut models primaries and transfer as *separate* CICP axes, but an upstream
//! like chromahash treats a single `Gamut` as a fixed bundle — e.g. `Bt2020`
//! means "BT.2020 primaries **and** PQ **and** Reinhard@203". A [`SourceProfile`]
//! is that bundle: it carries the `(gamut, transfer)` pair, the convenience
//! gamma-RGB → OKLab pipeline, and accessors that decompose it into
//! [`ColourPrimaries`] / [`TransferCharacteristics`].
//!
//! The HDR (BT.2020 PQ) path folds its Reinhard tone map into the encoder-exact
//! transfer (see [`crate::transfer::bt2020_pq_to_sdr`]); the general-purpose
//! tone-curve toolkit is the separate `gamut-tonemap` crate. Chromatic adaptation
//! needs no field here — it is already baked into the per-gamut `M1` matrix
//! (ProPhoto's D50→D65; see [`crate::matrix`]).

use crate::cicp::{ColourPrimaries, TransferCharacteristics};
use crate::oklab::{Gamut, linear_rgb_to_oklab};
use crate::transfer::{adobe_rgb_eotf, bt2020_pq_to_sdr, prophoto_rgb_eotf, srgb_eotf};

/// The encoder-exact per-channel transfer a [`SourceProfile`] linearizes with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceTransfer {
    /// sRGB EOTF (IEC 61966-2-1).
    Srgb,
    /// Adobe RGB, pure `x^2.2`.
    AdobeRgb,
    /// ProPhoto RGB, pure `x^1.8`.
    ProPhotoRgb,
    /// BT.2020 PQ inverse EOTF → nits → Reinhard@203 (tone-mapped to SDR).
    Bt2020Pq,
}

impl SourceTransfer {
    /// Apply the gamma-encoded → linear (SDR) transfer to one channel.
    #[must_use]
    pub fn eotf(self, x: f64) -> f64 {
        match self {
            SourceTransfer::Srgb => srgb_eotf(x),
            SourceTransfer::AdobeRgb => adobe_rgb_eotf(x),
            SourceTransfer::ProPhotoRgb => prophoto_rgb_eotf(x),
            SourceTransfer::Bt2020Pq => bt2020_pq_to_sdr(x),
        }
    }

    /// The CICP [`TransferCharacteristics`] code point, when one exists. Adobe
    /// RGB and ProPhoto RGB have no CICP transfer code point.
    #[must_use]
    pub fn cicp(self) -> Option<TransferCharacteristics> {
        match self {
            SourceTransfer::Srgb => Some(TransferCharacteristics::Srgb),
            SourceTransfer::Bt2020Pq => Some(TransferCharacteristics::Pq),
            SourceTransfer::AdobeRgb | SourceTransfer::ProPhotoRgb => None,
        }
    }
}

/// A source colour profile: a [`Gamut`] plus its encoder-exact [`SourceTransfer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceProfile {
    /// The colour-science gamut (selects the `M1` matrix / primaries).
    pub gamut: Gamut,
    /// The encoder-exact transfer used to linearize source samples.
    pub transfer: SourceTransfer,
}

impl SourceProfile {
    /// sRGB: BT.709 primaries + sRGB transfer.
    pub const SRGB: Self = Self {
        gamut: Gamut::Srgb,
        transfer: SourceTransfer::Srgb,
    };
    /// Display P3: DCI-P3 primaries + sRGB transfer.
    pub const DISPLAY_P3: Self = Self {
        gamut: Gamut::DisplayP3,
        transfer: SourceTransfer::Srgb,
    };
    /// Adobe RGB (1998): Adobe primaries + pure `x^2.2`.
    pub const ADOBE_RGB: Self = Self {
        gamut: Gamut::AdobeRgb,
        transfer: SourceTransfer::AdobeRgb,
    };
    /// BT.2020: BT.2020 primaries + PQ + Reinhard@203.
    pub const BT2020: Self = Self {
        gamut: Gamut::Bt2020,
        transfer: SourceTransfer::Bt2020Pq,
    };
    /// ProPhoto RGB: ProPhoto primaries (Bradford D50→D65) + pure `x^1.8`.
    pub const PROPHOTO_RGB: Self = Self {
        gamut: Gamut::ProPhotoRgb,
        transfer: SourceTransfer::ProPhotoRgb,
    };

    /// The CICP [`ColourPrimaries`] code point, when one exists. Adobe RGB and
    /// ProPhoto RGB have no CICP primaries code point.
    #[must_use]
    pub fn colour_primaries(self) -> Option<ColourPrimaries> {
        match self.gamut {
            // BT.709 and sRGB share primaries (CICP code point 1).
            Gamut::Srgb => Some(ColourPrimaries::Bt709),
            Gamut::DisplayP3 => Some(ColourPrimaries::DisplayP3),
            Gamut::Bt2020 => Some(ColourPrimaries::Bt2020),
            Gamut::AdobeRgb | Gamut::ProPhotoRgb => None,
        }
    }

    /// The CICP [`TransferCharacteristics`] code point, when one exists.
    #[must_use]
    pub fn transfer_characteristics(self) -> Option<TransferCharacteristics> {
        self.transfer.cicp()
    }

    /// Linearize one gamma-encoded channel with this profile's transfer.
    #[must_use]
    pub fn eotf(self, x: f64) -> f64 {
        self.transfer.eotf(x)
    }

    /// Convert a gamma-encoded source RGB triple to OKLab: apply the transfer
    /// per channel, then project through the gamut's `M1` matrix.
    #[must_use]
    pub fn gamma_rgb_to_oklab(self, rgb: [f64; 3]) -> [f64; 3] {
        let linear = [self.eotf(rgb[0]), self.eotf(rgb[1]), self.eotf(rgb[2])];
        linear_rgb_to_oklab(linear, self.gamut)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn srgb_decomposes_to_cicp_axes() {
        let p = SourceProfile::SRGB;
        assert_eq!(p.colour_primaries(), Some(ColourPrimaries::Bt709));
        assert_eq!(
            p.transfer_characteristics(),
            Some(TransferCharacteristics::Srgb)
        );
    }

    #[test]
    fn bt2020_decomposes_to_primaries_and_pq() {
        let p = SourceProfile::BT2020;
        assert_eq!(p.colour_primaries(), Some(ColourPrimaries::Bt2020));
        assert_eq!(
            p.transfer_characteristics(),
            Some(TransferCharacteristics::Pq)
        );
        // The Reinhard tone map folded into the BT.2020 transfer is exercised by
        // `eotf_is_encoder_exact_per_gamut` (eotf == bt2020_pq_to_sdr).
    }

    #[test]
    fn adobe_and_prophoto_have_no_cicp_code_points() {
        for p in [SourceProfile::ADOBE_RGB, SourceProfile::PROPHOTO_RGB] {
            assert_eq!(p.colour_primaries(), None);
            assert_eq!(p.transfer_characteristics(), None);
        }
        // Display P3 reuses the sRGB transfer but its own primaries.
        assert_eq!(
            SourceProfile::DISPLAY_P3.colour_primaries(),
            Some(ColourPrimaries::DisplayP3)
        );
    }

    #[test]
    fn eotf_is_encoder_exact_per_gamut() {
        // Adobe is pure x^2.2, ProPhoto pure x^1.8 — distinct at the same input.
        assert_eq!(SourceProfile::ADOBE_RGB.eotf(0.5), 0.5_f64.powf(2.2));
        assert_eq!(SourceProfile::PROPHOTO_RGB.eotf(0.5), 0.5_f64.powf(1.8));
        // BT.2020 folds in the PQ + Reinhard tone map.
        assert_eq!(SourceProfile::BT2020.eotf(0.5), bt2020_pq_to_sdr(0.5));
    }

    /// Golden `gamma_rgb` vectors transcribed from chromahash
    /// `spec/test-vectors/unit-color.json` (MIT OR Apache-2.0).
    #[test]
    fn matches_chromahash_gamma_pipeline_vectors() {
        let cases: &[([f64; 3], [f64; 3])] = &[
            // gamma_red_srgb (sRGB EOTF of 1/0 is 1/0, so equals linear red).
            (
                [1.0, 0.0, 0.0],
                [0.6279553606145517, 0.224863061065974, 0.12584629853073515],
            ),
            // gamma_mid_srgb.
            (
                [0.5, 0.5, 0.5],
                [
                    0.5981807266228486,
                    0.000000000048424320109319297,
                    0.000000022296533230825588,
                ],
            ),
        ];
        for &(gamma_rgb, want) in cases {
            let got = SourceProfile::SRGB.gamma_rgb_to_oklab(gamma_rgb);
            for (i, (&g, &w)) in got.iter().zip(want.iter()).enumerate() {
                assert!((g - w).abs() < 1e-9, "oklab[{i}] = {g}, want {w}");
            }
        }
    }
}
