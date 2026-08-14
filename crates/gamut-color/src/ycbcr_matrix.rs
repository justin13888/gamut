//! The general CICP (ITU-T H.273 §8.3) RGB ↔ YCbCr transform for the **non-constant-luminance**
//! matrices — BT.601, BT.709 and BT.2020 — at either signal range.
//!
//! [`crate::ycbcr`] is deliberately *not* this: it is BT.601 only, and its limited-range arm
//! reproduces libwebp's integer constants bit-for-bit because a WebP file must decode identically
//! under `dwebp`. That is a compatibility contract with one implementation, not the spec formula.
//! This module is the spec formula, parameterized by matrix and range, for the codecs (AVIF/AV1)
//! whose conformance target is H.273 itself. The two agree exactly for full-range BT.601 — the JFIF
//! convention — which the tests assert.
//!
//! # The transform
//!
//! H.273 §8.3 derives everything from the matrix's two luma weights `KR` and `KB`
//! (`KG = 1 − KR − KB`), over `R'G'B'` in `0..=1`:
//!
//! ```text
//! Y'  = KR·R' + KG·G' + KB·B'
//! Cb  = (B' − Y') / (2·(1 − KB))
//! Cr  = (R' − Y') / (2·(1 − KR))
//! ```
//!
//! and quantizes to 8 bits with the range's excursions: full range scales luma and chroma by 255,
//! studio ("limited") range by 219 and 224 with a `+16` luma pedestal. Chroma always centres on 128.
//!
//! Coefficients are derived once into 16-bit fixed point by [`YcbcrMatrix::new`] and then applied
//! with integer arithmetic only, so a conversion is reproducible across platforms (unlike the
//! `f64` colour science in [`crate::matrix`]).

use gamut_core::{Error, Result};

use crate::cicp::{ColorRange, MatrixCoefficients};
use crate::clip_pixel8;

/// Fractional bits of the fixed-point coefficients.
const FIX: u32 = 16;
/// The fixed-point scale, `2^FIX`.
const ONE: f64 = (1u32 << FIX) as f64;
/// Rounding addend: `0.5` in the fixed-point scale.
const HALF: i32 = 1 << (FIX - 1);
/// The chroma centre (`128`) pre-scaled to the fixed-point domain, with rounding folded in.
const CHROMA_BIAS: i32 = (128 << FIX) + HALF;

/// The `(KR, KB)` luma weights of a non-constant-luminance matrix (H.273 Table 4), or `None` for a
/// code point this module does not transform: `Identity` carries R'G'B' with no transform at all,
/// `Unspecified` names no matrix, and `YCgCo` is a different (integer-lifting) construction.
fn luma_weights(matrix: MatrixCoefficients) -> Option<(f64, f64)> {
    match matrix {
        MatrixCoefficients::Bt709 => Some((0.2126, 0.0722)),
        MatrixCoefficients::Bt601 => Some((0.299, 0.114)),
        MatrixCoefficients::Bt2020Ncl => Some((0.2627, 0.0593)),
        _ => None,
    }
}

/// Rounds a real coefficient into the 16-bit fixed-point domain.
fn fixed(value: f64) -> i32 {
    (value * ONE).round() as i32
}

/// A prepared 8-bit RGB ↔ YCbCr transform for one CICP matrix and [`ColorRange`].
///
/// Build it once per image with [`YcbcrMatrix::new`], then call [`YcbcrMatrix::forward`] /
/// [`YcbcrMatrix::inverse`] per pixel — the fallible matrix check happens only at construction, and
/// the per-pixel path is branch-free integer arithmetic.
///
/// # Examples
///
/// ```
/// use gamut_color::{ColorRange, MatrixCoefficients, YcbcrMatrix};
///
/// let m = YcbcrMatrix::new(MatrixCoefficients::Bt709, ColorRange::Full)?;
/// // Neutral grey stays neutral: no chroma, luma unchanged.
/// assert_eq!(m.forward(128, 128, 128), (128, 128, 128));
/// # Ok::<(), gamut_core::Error>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct YcbcrMatrix {
    /// Forward luma row `(R, G, B)`.
    fwd_y: [i32; 3],
    /// Forward Cb row `(R, G, B)`.
    fwd_cb: [i32; 3],
    /// Forward Cr row `(R, G, B)`.
    fwd_cr: [i32; 3],
    /// Luma pedestal added by the forward transform (`16` for studio range, `0` for full),
    /// pre-scaled, with the rounding addend folded in.
    fwd_luma_bias: i32,
    /// Inverse luma gain (`255/219` for studio range, `1` for full).
    inv_y: i32,
    /// Inverse chroma terms: `Cr→R`, `Cb→G`, `Cr→G`, `Cb→B`.
    inv_c: [i32; 4],
    /// Luma pedestal the inverse transform removes before scaling.
    inv_luma_offset: i32,
}

impl YcbcrMatrix {
    /// Prepares the transform for `matrix` at `range`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Unsupported`] for a matrix this module does not transform:
    /// [`MatrixCoefficients::Identity`] (which carries R'G'B' directly — see
    /// [`Planar8::from_rgb8_identity`](crate::Planar8::from_rgb8_identity)),
    /// [`MatrixCoefficients::Unspecified`], and [`MatrixCoefficients::YCgCo`].
    pub fn new(matrix: MatrixCoefficients, range: ColorRange) -> Result<Self> {
        let (kr, kb) = luma_weights(matrix).ok_or_else(|| {
            Error::unsupported(
                env!("CARGO_PKG_NAME"),
                "no non-constant-luminance YCbCr transform for these matrix coefficients",
            )
        })?;
        let kg = 1.0 - kr - kb;
        // 8-bit excursions (H.273 §8.3): full range spends all 255 codes on both luma and chroma;
        // studio range spends 219 on luma above a 16-code pedestal, and 224 on chroma about 128.
        let (luma_excursion, chroma_excursion, pedestal) = match range {
            ColorRange::Full => (255.0, 255.0, 0),
            ColorRange::Limited => (219.0, 224.0, 16),
        };
        // The R'G'B' inputs are themselves `sample / 255`, so every forward row carries a `/255`
        // that cancels against the excursion.
        let fy = luma_excursion / 255.0;
        let fc = chroma_excursion / 255.0;
        let (cb_denom, cr_denom) = (2.0 * (1.0 - kb), 2.0 * (1.0 - kr));

        Ok(Self {
            fwd_y: [fixed(fy * kr), fixed(fy * kg), fixed(fy * kb)],
            fwd_cb: [
                fixed(fc * -kr / cb_denom),
                fixed(fc * -kg / cb_denom),
                fixed(fc * (1.0 - kb) / cb_denom),
            ],
            fwd_cr: [
                fixed(fc * (1.0 - kr) / cr_denom),
                fixed(fc * -kg / cr_denom),
                fixed(fc * -kb / cr_denom),
            ],
            fwd_luma_bias: (pedestal << FIX) + HALF,
            inv_y: fixed(255.0 / luma_excursion),
            inv_c: [
                fixed(255.0 / chroma_excursion * cr_denom),
                fixed(255.0 / chroma_excursion * -cb_denom * kb / kg),
                fixed(255.0 / chroma_excursion * -cr_denom * kr / kg),
                fixed(255.0 / chroma_excursion * cb_denom),
            ],
            inv_luma_offset: pedestal,
        })
    }

    /// Converts one 8-bit R'G'B' triple to Y'CbCr.
    #[must_use]
    pub fn forward(self, r: u8, g: u8, b: u8) -> (u8, u8, u8) {
        let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
        let dot = |m: [i32; 3]| m[0] * r + m[1] * g + m[2] * b;
        let y = (dot(self.fwd_y) + self.fwd_luma_bias) >> FIX;
        let cb = (dot(self.fwd_cb) + CHROMA_BIAS) >> FIX;
        let cr = (dot(self.fwd_cr) + CHROMA_BIAS) >> FIX;
        (clip_pixel8(y), clip_pixel8(cb), clip_pixel8(cr))
    }

    /// Converts one 8-bit Y'CbCr triple back to R'G'B'. Out-of-gamut results clamp to `0..=255`,
    /// which is what a decoder must do with codes the encoder could never have produced.
    #[must_use]
    pub fn inverse(self, y: u8, cb: u8, cr: u8) -> (u8, u8, u8) {
        let luma = self.inv_y * (i32::from(y) - self.inv_luma_offset);
        let (cb, cr) = (i32::from(cb) - 128, i32::from(cr) - 128);
        let r = (luma + self.inv_c[0] * cr + HALF) >> FIX;
        let g = (luma + self.inv_c[1] * cb + self.inv_c[2] * cr + HALF) >> FIX;
        let b = (luma + self.inv_c[3] * cb + HALF) >> FIX;
        (clip_pixel8(r), clip_pixel8(g), clip_pixel8(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ycbcr::{rgb_to_ycbcr, ycbcr_to_rgb};

    /// Every `(matrix, range)` this module supports.
    const SUPPORTED: [MatrixCoefficients; 3] = [
        MatrixCoefficients::Bt601,
        MatrixCoefficients::Bt709,
        MatrixCoefficients::Bt2020Ncl,
    ];

    #[test]
    fn unsupported_matrices_are_rejected() {
        // Identity is not "the transform with KR = KB = 0" — it is the absence of one, and silently
        // treating it as a matrix would corrupt the lossless AVIF path.
        for m in [
            MatrixCoefficients::Identity,
            MatrixCoefficients::Unspecified,
            MatrixCoefficients::YCgCo,
        ] {
            assert!(YcbcrMatrix::new(m, ColorRange::Full).is_err(), "{m:?}");
        }
    }

    #[test]
    fn full_range_bt601_reproduces_the_jfif_path_exactly() {
        // The JFIF conversion in `crate::ycbcr` is full-range BT.601 with hand-transcribed
        // constants; deriving the same matrix from KR/KB must land on exactly the same integers,
        // for every input. That pins the derivation formulas against an independent transcription.
        let m = YcbcrMatrix::new(MatrixCoefficients::Bt601, ColorRange::Full).unwrap();
        for r in (0..=255u8).step_by(5) {
            for g in (0..=255u8).step_by(7) {
                for b in (0..=255u8).step_by(11) {
                    assert_eq!(
                        m.forward(r, g, b),
                        rgb_to_ycbcr(r, g, b, ColorRange::Full),
                        "forward at ({r},{g},{b})"
                    );
                }
            }
        }
        for y in (0..=255u8).step_by(5) {
            for cb in (0..=255u8).step_by(7) {
                for cr in (0..=255u8).step_by(11) {
                    assert_eq!(
                        m.inverse(y, cb, cr),
                        ycbcr_to_rgb(y, cb, cr, ColorRange::Full),
                        "inverse at ({y},{cb},{cr})"
                    );
                }
            }
        }
    }

    #[test]
    fn primaries_land_on_the_h273_reference_values() {
        // Hand-computed from H.273 §8.3 with the Table 4 weights. Pure red under BT.709
        // (KR = 0.2126) has Y = round(255·0.2126) = 54, Cr at its positive extreme (255) and Cb at
        // its negative one: Cb = round(255·(0 − 0.2126)/(2·0.9278)) + 128 = round(−29.2) + 128 = 99.
        let m709 = YcbcrMatrix::new(MatrixCoefficients::Bt709, ColorRange::Full).unwrap();
        assert_eq!(m709.forward(255, 0, 0), (54, 99, 255));
        // Pure blue: Y = round(255·0.0722) = 18, Cb at +255, Cr = round(255·(0 − 0.0722)/(2·0.7874))
        // + 128 = round(−11.7) + 128 = 116.
        assert_eq!(m709.forward(0, 0, 255), (18, 255, 116));

        // BT.601 (KR = 0.299): pure red → Y = round(255·0.299) = 76.
        let m601 = YcbcrMatrix::new(MatrixCoefficients::Bt601, ColorRange::Full).unwrap();
        assert_eq!(m601.forward(255, 0, 0).0, 76);
        // BT.2020 (KR = 0.2627): pure red → Y = round(255·0.2627) = 67.
        let m2020 = YcbcrMatrix::new(MatrixCoefficients::Bt2020Ncl, ColorRange::Full).unwrap();
        assert_eq!(m2020.forward(255, 0, 0).0, 67);
        // The three matrices really are distinct — a mutated weight table that collapsed them
        // would still pass a round-trip test.
        assert_ne!(m709.forward(255, 0, 0), m601.forward(255, 0, 0));
        assert_ne!(m709.forward(255, 0, 0), m2020.forward(255, 0, 0));
    }

    #[test]
    fn studio_range_uses_the_16_235_pedestal_and_excursions() {
        for matrix in SUPPORTED {
            let m = YcbcrMatrix::new(matrix, ColorRange::Limited).unwrap();
            // Black → 16, white → 235, both with neutral chroma (H.273's studio excursions).
            assert_eq!(m.forward(0, 0, 0), (16, 128, 128), "{matrix:?} black");
            assert_eq!(
                m.forward(255, 255, 255),
                (235, 128, 128),
                "{matrix:?} white"
            );
            // …and the inverse removes the pedestal again.
            assert_eq!(m.inverse(16, 128, 128), (0, 0, 0), "{matrix:?} black back");
            assert_eq!(
                m.inverse(235, 128, 128),
                (255, 255, 255),
                "{matrix:?} white back"
            );
        }
    }

    #[test]
    fn full_range_spans_0_to_255() {
        for matrix in SUPPORTED {
            let m = YcbcrMatrix::new(matrix, ColorRange::Full).unwrap();
            assert_eq!(m.forward(0, 0, 0), (0, 128, 128), "{matrix:?} black");
            assert_eq!(
                m.forward(255, 255, 255),
                (255, 128, 128),
                "{matrix:?} white"
            );
        }
    }

    #[test]
    fn round_trip_is_within_one_code_per_channel() {
        // 8-bit YCbCr is not a bijection of 8-bit RGB — the transform is lossy by quantization
        // alone. Bound the error rather than assert equality, over a grid dense enough to catch a
        // sign or row swap (which would blow past ±2 immediately).
        for matrix in SUPPORTED {
            for range in [ColorRange::Full, ColorRange::Limited] {
                let m = YcbcrMatrix::new(matrix, range).unwrap();
                // Studio range discards 36 of the 256 luma codes, so its round-trip error is
                // larger; full range is tight.
                let tolerance = match range {
                    ColorRange::Full => 1,
                    ColorRange::Limited => 2,
                };
                for r in (0..=255u8).step_by(15) {
                    for g in (0..=255u8).step_by(17) {
                        for b in (0..=255u8).step_by(19) {
                            let (y, cb, cr) = m.forward(r, g, b);
                            let (r2, g2, b2) = m.inverse(y, cb, cr);
                            for (src, back, name) in [(r, r2, "R"), (g, g2, "G"), (b, b2, "B")] {
                                assert!(
                                    src.abs_diff(back) <= tolerance,
                                    "{matrix:?} {range:?} {name} at ({r},{g},{b}): {src} → {back}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn out_of_gamut_chroma_clamps_instead_of_wrapping() {
        // Extreme chroma with mid luma is reachable in a corrupt or hostile stream; the inverse
        // must saturate, not wrap around through the `as u8` cast.
        let m = YcbcrMatrix::new(MatrixCoefficients::Bt709, ColorRange::Full).unwrap();
        assert_eq!(m.inverse(128, 0, 255).0, 255);
        assert_eq!(m.inverse(128, 255, 0).2, 255);
        assert_eq!(m.inverse(128, 255, 0).0, 0);
    }
}
