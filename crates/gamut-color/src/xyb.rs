//! The JPEG XL **XYB** opsin colour space: linear sRGB ⇄ XYB and the scaled-XYB byte encoding.
//!
//! XYB is JPEG XL's perceptual working space (ISO/IEC 18181-1; the pre-ISO Committee Draft is
//! vendored as `references/jxl/1908.03565.pdf`): linear sRGB is mixed by the **opsin absorbance
//! matrix** into an LMS-like basis, biased and cube-rooted (`x ↦ ∛(x + b) − ∛b`), and folded into
//! opponent channels `X = (L′ − M′)/2`, `Y = (L′ + M′)/2`, `B = S′`. The exact constants are
//! **frozen** by the reference implementation (libjxl 0.12.0, `lib/jxl/cms/opsin_params.h` — the
//! version this workspace already pins as its JXL oracle) and transcribed with provenance notes in
//! `references/color/README.md`.
//!
//! [`scale_xyb`]/[`unscale_xyb`] are libjxl's `ScaleXYB` affine byte mapping: per-channel offsets
//! and scales that place the sRGB cube inside `[0, 1]³`, with the **third stored channel being
//! `B − Y`**, not `B`. This is the sample encoding jpegli-style XYB JPEGs store, and the one the
//! embedded ICC profile of `gamut-jpeg`'s XYB mode describes.
//!
//! Tier-1 determinism: `std` `f64::cbrt`/`powi` — correctly rounded per platform libm, not
//! bit-reproducible across platforms (the crate-wide policy; see `STATUS.md`).

use crate::linalg::matvec3;

/// The opsin absorbance matrix (linear sRGB → mixed LMS), frozen by libjxl. Each row sums to 1
/// (the middle/last entries are defined upstream as `1 − the others`, so the sums hold to an ulp),
/// making opsin white `(1, 1, 1)`.
// The literals are upstream's verbatim (they carry more digits than f64 resolves; truncating
// them here would break the transcribed-with-citation contract of references/color/README.md).
#[allow(clippy::excessive_precision)]
pub const OPSIN_ABSORBANCE: [[f64; 3]; 3] = [
    [0.30, 1.0 - 0.078 - 0.30, 0.078],
    [0.23, 1.0 - 0.078 - 0.23, 0.078],
    [
        0.243_422_689_245_478_19,
        0.204_767_444_244_968_21,
        1.0 - 0.243_422_689_245_478_19 - 0.204_767_444_244_968_21,
    ],
];

/// The opsin absorbance bias `b` (identical for all three channels), frozen by libjxl.
pub const OPSIN_ABSORBANCE_BIAS: f64 = 0.003_793_073_255_275_449_3;

/// The frozen **inverse** opsin absorbance matrix (mixed LMS → linear sRGB), transcribed from the
/// reference implementation rather than re-derived: the decode direction is normative and these
/// exact literals are what every XYB decoder applies.
pub const OPSIN_INVERSE: [[f64; 3]; 3] = [
    [
        11.031_566_901_960_783,
        -9.866_943_921_568_629,
        -0.164_622_996_470_588_26,
    ],
    [
        -3.254_147_380_392_157,
        4.418_770_392_156_863,
        -0.164_622_996_470_588_26,
    ],
    [
        -3.658_851_286_274_509_7,
        2.712_923_047_058_823_5,
        1.945_928_239_215_686_3,
    ],
];

/// Per-channel offsets of the scaled-XYB byte encoding (libjxl `kScaledXYBOffset`); channel 2
/// applies to `B − Y`.
pub const SCALED_XYB_OFFSET: [f64; 3] = [0.015_386_134, 0.0, 0.277_704_59];

/// Per-channel scales of the scaled-XYB byte encoding (libjxl `kScaledXYBScale`); channel 2
/// applies to `B − Y`.
pub const SCALED_XYB_SCALE: [f64; 3] = [22.995_788_804, 1.183_000_077, 1.502_141_333];

/// Linear sRGB (nominal `[0, 1]`, out-of-range values pass through the signed cube root) → XYB.
#[must_use]
pub fn linear_srgb_to_xyb(rgb: [f64; 3]) -> [f64; 3] {
    let mixed = matvec3(&OPSIN_ABSORBANCE, rgb);
    let cbrt_bias = OPSIN_ABSORBANCE_BIAS.cbrt();
    let l = (mixed[0] + OPSIN_ABSORBANCE_BIAS).cbrt() - cbrt_bias;
    let m = (mixed[1] + OPSIN_ABSORBANCE_BIAS).cbrt() - cbrt_bias;
    let s = (mixed[2] + OPSIN_ABSORBANCE_BIAS).cbrt() - cbrt_bias;
    [(l - m) / 2.0, (l + m) / 2.0, s]
}

/// XYB → linear sRGB, via the frozen [`OPSIN_INVERSE`] matrix (the normative decode direction:
/// un-mix the opponent channels, cube, remove the bias, then the inverse matrix).
#[must_use]
pub fn xyb_to_linear_srgb(xyb: [f64; 3]) -> [f64; 3] {
    let cbrt_bias = OPSIN_ABSORBANCE_BIAS.cbrt();
    let l = xyb[1] + xyb[0] + cbrt_bias;
    let m = xyb[1] - xyb[0] + cbrt_bias;
    let s = xyb[2] + cbrt_bias;
    let mixed = [
        l.powi(3) - OPSIN_ABSORBANCE_BIAS,
        m.powi(3) - OPSIN_ABSORBANCE_BIAS,
        s.powi(3) - OPSIN_ABSORBANCE_BIAS,
    ];
    matvec3(&OPSIN_INVERSE, mixed)
}

/// XYB → the scaled byte encoding `[0, 1]³` (libjxl `ScaleXYB`): per-channel affine with the
/// third stored channel being `B − Y`, each clamped to `[0, 1]`. Multiply by 255 and round for
/// the 8-bit samples an XYB JPEG stores.
#[must_use]
pub fn scale_xyb(xyb: [f64; 3]) -> [f64; 3] {
    let stored = [xyb[0], xyb[1], xyb[2] - xyb[1]];
    let mut out = [0.0; 3];
    for i in 0..3 {
        out[i] = ((stored[i] + SCALED_XYB_OFFSET[i]) * SCALED_XYB_SCALE[i]).clamp(0.0, 1.0);
    }
    out
}

/// The inverse of [`scale_xyb`] (exact for in-gamut, unclamped values): scaled `[0, 1]³` samples
/// back to XYB, restoring `B` from the stored `B − Y`.
#[must_use]
pub fn unscale_xyb(scaled: [f64; 3]) -> [f64; 3] {
    let mut stored = [0.0; 3];
    for i in 0..3 {
        stored[i] = scaled[i] / SCALED_XYB_SCALE[i] - SCALED_XYB_OFFSET[i];
    }
    [stored[0], stored[1], stored[2] + stored[1]]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::linalg::mat_mul3;

    #[test]
    fn absorbance_rows_sum_to_one() {
        // Upstream defines each row's middle/last entry as 1 − the others, so each sum is 1 up to
        // one ulp of re-association (the summation order differs from the subtraction order) and
        // opsin white is (1, 1, 1).
        for row in &OPSIN_ABSORBANCE {
            assert!((row.iter().sum::<f64>() - 1.0).abs() < 1e-15, "{row:?}");
        }
    }

    #[test]
    fn inverse_matrix_inverts_the_absorbance_matrix() {
        // The frozen inverse literals were published from f32-rounded forward entries, so the
        // product is the identity to ~1e-6, not machine epsilon.
        let product = mat_mul3(&OPSIN_INVERSE, &OPSIN_ABSORBANCE);
        for (i, row) in product.iter().enumerate() {
            for (j, &v) in row.iter().enumerate() {
                let expected = if i == j { 1.0 } else { 0.0 };
                assert!((v - expected).abs() < 1e-6, "product[{i}][{j}] = {v}");
            }
        }
    }

    #[test]
    fn black_and_white_hit_their_analytic_values() {
        // Black: every channel is ∛b − ∛b = 0. White: mixed LMS is exactly (1,1,1) (row sums),
        // so X = 0 and Y = B = ∛(1 + b) − ∛b — computed here independently of the transform.
        let black = linear_srgb_to_xyb([0.0, 0.0, 0.0]);
        assert_eq!(black, [0.0, 0.0, 0.0]);
        let white = linear_srgb_to_xyb([1.0, 1.0, 1.0]);
        let expected = (1.0 + OPSIN_ABSORBANCE_BIAS).cbrt() - OPSIN_ABSORBANCE_BIAS.cbrt();
        assert!((white[0]).abs() < 1e-12, "white X = {}", white[0]);
        assert!((white[1] - expected).abs() < 1e-12);
        assert!((white[2] - expected).abs() < 1e-12);
    }

    #[test]
    fn round_trips_are_tight_over_a_value_grid() {
        // Forward → inverse must reproduce the input to the precision the f32-provenance inverse
        // matrix allows (~1e-6); scale/unscale is affine and exact to f64 rounding for in-range
        // values.
        for r in 0..=4 {
            for g in 0..=4 {
                for b in 0..=4 {
                    let rgb = [f64::from(r) / 4.0, f64::from(g) / 4.0, f64::from(b) / 4.0];
                    let xyb = linear_srgb_to_xyb(rgb);
                    let back = xyb_to_linear_srgb(xyb);
                    for i in 0..3 {
                        assert!((back[i] - rgb[i]).abs() < 1e-6, "rgb {rgb:?} -> {back:?}");
                    }
                    let scaled = scale_xyb(xyb);
                    let unscaled = unscale_xyb(scaled);
                    for i in 0..3 {
                        // The affine pair cancels catastrophically near the offsets (X and B − Y
                        // both sit within ~1e-8 of theirs at the gamut edge), so ~1e-6 is the
                        // honest bound, comfortably under the 1/255 quantization it feeds.
                        assert!(
                            (unscaled[i] - xyb[i]).abs() < 1e-6,
                            "xyb {xyb:?} -> {unscaled:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn scaled_boundaries_pin_the_offsets_scales_and_the_b_minus_y_store() {
        // White: X = 0 → s0 = 0.015386134·22.995788804 ≈ 0.3538; Y ≈ 0.84481 → s1 ≈ 0.99945;
        // B − Y = 0 → s2 = 0.27770459·1.502141333 ≈ 0.41714. The sRGB blue primary maximizes
        // B − Y and lands at s2 ≈ 1.0005, clamped to exactly 1.0 — together these pin each offset,
        // each scale, and the B − Y subtraction (a swapped channel or a stored plain B lands far
        // outside the tolerances).
        let white = scale_xyb(linear_srgb_to_xyb([1.0, 1.0, 1.0]));
        assert!(
            (white[0] - 0.353_84).abs() < 1e-3,
            "white s0 = {}",
            white[0]
        );
        assert!(
            (white[1] - 0.999_45).abs() < 1e-3,
            "white s1 = {}",
            white[1]
        );
        assert!(
            (white[2] - 0.417_14).abs() < 1e-3,
            "white s2 = {}",
            white[2]
        );
        let blue = scale_xyb(linear_srgb_to_xyb([0.0, 0.0, 1.0]));
        assert!(
            (blue[2] - 1.0).abs() < 1e-6,
            "blue s2 = {} must land on the top of the range",
            blue[2]
        );
        // And the primaries stay inside [0, 1] on every channel (the mapping's design goal).
        for rgb in [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] {
            let s = scale_xyb(linear_srgb_to_xyb(rgb));
            assert!(
                s.iter().all(|&v| (0.0..=1.0).contains(&v)),
                "{rgb:?}: {s:?}"
            );
        }
    }
}
