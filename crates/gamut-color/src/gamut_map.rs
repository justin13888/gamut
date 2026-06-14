//! sRGB gamut membership and a hue-preserving soft gamut clamp.
//!
//! [`soft_gamut_clamp`] projects an out-of-gamut OKLab colour along the straight
//! OKLab segment toward an achromatic anchor (lightness blended toward mid-gray),
//! shrinking `a`/`b` together so hue is preserved, via a fixed 16-iteration
//! bisection. The iteration count is pinned for reproducibility. With
//! `l_blend = 0` it reduces to a constant-`L` chroma-only clamp.

use crate::oklab::oklab_to_linear_srgb;

/// Number of bisection steps. Pinned (no early exit) for reproducibility.
const CLAMP_ITERATIONS: u32 = 16;

/// Whether every channel of a **linear sRGB** colour lies in `[0, 1]` (exact).
#[must_use]
pub fn in_gamut(rgb: [f64; 3]) -> bool {
    rgb[0] >= 0.0
        && rgb[0] <= 1.0
        && rgb[1] >= 0.0
        && rgb[1] <= 1.0
        && rgb[2] >= 0.0
        && rgb[2] <= 1.0
}

/// Hue-preserving soft gamut clamp of an OKLab colour `[L, a, b]` into the sRGB gamut.
///
/// An in-gamut input is returned unchanged. Otherwise the colour is projected
/// toward the achromatic anchor `(L + l_blend·(0.5 − L), 0, 0)` — which is always
/// in gamut — and bisection over `t ∈ [0, 1]` converges onto the gamut surface
/// in exactly `CLAMP_ITERATIONS` (16) steps. `a` and `b` shrink by the same factor,
/// so hue is preserved; lightness drifts toward 0.5 in proportion to how far out
/// of gamut the colour was (`l_blend = 0` keeps `L` fixed).
///
/// `lab` matches the `[L, a, b]` layout returned by [`linear_rgb_to_oklab`].
/// Precondition: `lab[0]` (L) should be in `[0, 1]`.
///
/// [`linear_rgb_to_oklab`]: crate::oklab::linear_rgb_to_oklab
#[must_use]
pub fn soft_gamut_clamp(lab: [f64; 3], l_blend: f64) -> [f64; 3] {
    if in_gamut(oklab_to_linear_srgb(lab)) {
        return lab;
    }

    let [l, a, b] = lab;
    let anchor_l = l + l_blend * (0.5 - l);

    let mut lo = 0.0_f64;
    let mut hi = 1.0_f64;
    for _ in 0..CLAMP_ITERATIONS {
        let mid = (lo + hi) / 2.0;
        let test = [l + (anchor_l - l) * mid, a * (1.0 - mid), b * (1.0 - mid)];
        if in_gamut(oklab_to_linear_srgb(test)) {
            hi = mid;
        } else {
            lo = mid;
        }
    }

    // `hi` is the in-gamut side of the final bracket (hi = 1 is in gamut, and
    // every assignment to `hi` followed a successful in-gamut test).
    [l + (anchor_l - l) * hi, a * (1.0 - hi), b * (1.0 - hi)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_gamut_boundaries() {
        assert!(in_gamut([0.0, 0.0, 0.0]));
        assert!(in_gamut([1.0, 1.0, 1.0]));
        assert!(in_gamut([0.5, 0.3, 0.8]));
        assert!(!in_gamut([1.1, 0.5, 0.5]));
        assert!(!in_gamut([0.5, -0.1, 0.5]));
    }

    #[test]
    fn in_gamut_color_passes_through() {
        for blend in [0.0, 0.35, 0.5] {
            assert_eq!(soft_gamut_clamp([0.5, 0.0, 0.0], blend), [0.5, 0.0, 0.0]);
        }
    }

    #[test]
    fn blend_zero_preserves_lightness_and_hue() {
        let (l, a, b) = (0.5, 0.4, 0.0);
        assert!(!in_gamut(oklab_to_linear_srgb([l, a, b])));
        let [lo, ao, bo] = soft_gamut_clamp([l, a, b], 0.0);
        assert_eq!(lo, l, "L unchanged at blend 0");
        assert!(ao.abs() <= a.abs() + 1e-12, "chroma must not grow");
        assert!(bo.abs() <= 1e-12);
        assert!(in_gamut(oklab_to_linear_srgb([lo, ao, bo])));
    }

    #[test]
    fn preserves_hue_angle() {
        let (l, a, b) = (0.5, 0.3, -0.2);
        let [_, ao, bo] = soft_gamut_clamp([l, a, b], 0.35);
        // a and b shrink by the same factor → cross product (hue) stays zero.
        assert!((a * bo - b * ao).abs() < 1e-12);
        assert!(ao * a >= 0.0 && bo * b >= 0.0);
    }

    /// Golden vectors transcribed from chromahash `spec/test-vectors/unit-softgamutclamp.json`
    /// (MIT OR Apache-2.0). Bisection decisions depend only on coarse 2⁻¹⁶ midpoints,
    /// so Tier-1 `std` math reproduces chromahash's outputs to within this tolerance.
    #[test]
    fn matches_chromahash_clamp_vectors() {
        // (L, a, b, l_blend) → (L, a, b)
        let cases: &[([f64; 4], [f64; 3])] = &[
            ([0.5, 0.0, 0.0, 0.5], [0.5, 0.0, 0.0]),
            ([1.0, 0.0, 0.0, 0.5], [1.0, 0.0, 0.0]),
            ([0.0, 0.0, 0.0, 0.5], [0.0, 0.0, 0.0]),
            ([0.7, -0.1, 0.1, 0.5], [0.7, -0.1, 0.1]),
            ([0.5, 0.4, 0.2, 0.5], [0.5, 0.18203125, 0.091015625]),
            (
                [0.4, -0.1, -0.3, 0.5],
                [
                    0.42991485595703127,
                    -0.040170288085937506,
                    -0.12051086425781249,
                ],
            ),
            (
                [0.8, -0.05, 0.3, 0.5],
                [
                    0.7241188049316407,
                    -0.024706268310546876,
                    0.14823760986328124,
                ],
            ),
            ([0.5, 0.45, 0.0, 0.5], [0.5, 0.2026908874511719, 0.0]),
            ([0.5, 0.0, 0.45, 0.5], [0.5, 0.0, 0.1021728515625]),
            (
                [0.7, 0.25, 0.12, 0.5],
                [0.6762924194335938, 0.19073104858398438, 0.0915509033203125],
            ),
            (
                [0.4488, -0.0357, -0.3143, 0.5],
                [
                    0.45472265624999997,
                    -0.02744067077636719,
                    -0.24158551330566408,
                ],
            ),
            ([0.1, 0.0, 0.0, 0.5], [0.1, 0.0, 0.0]),
            ([0.9, 0.0, 0.0, 0.5], [0.9, 0.0, 0.0]),
        ];
        for &([l, a, b, blend], want) in cases {
            let got = soft_gamut_clamp([l, a, b], blend);
            for (i, (&g, &w)) in got.iter().zip(want.iter()).enumerate() {
                assert!(
                    (g - w).abs() < 1e-6,
                    "clamp({l},{a},{b},{blend})[{i}] = {g}, want {w}"
                );
            }
        }
    }
}
