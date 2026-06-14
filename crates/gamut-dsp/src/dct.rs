//! AV1 1-D inverse and forward discrete cosine transforms (DCT) for `TX` lengths 4/8/16/32/64.
//!
//! [`inverse_dct`] is a direct transcription of the normative inverse DCT process (AV1 §7.13.2.3),
//! built on the shared [`crate::butterfly`] primitives. It is the transform the decoder runs, so
//! it must be bit-exact; the unit tests pin it against an independent naive DCT-III reference.
//!
//! [`forward_dct`] is the encoder transform. AV1 specifies only the *inverse*; the forward is an
//! encoder choice, so this uses the straightforward unnormalized DCT-II (an O(n²) cosine sum on
//! the same [`COS128_LOOKUP`](crate::butterfly) table — deterministic integer arithmetic, not
//! floating point). Reconstruction correctness depends solely on the inverse and the dequant, so
//! any consistent forward is valid; quality tuning of the forward is deferred to the encoder's
//! rate-distortion layer. The per-pass normalization shifts that make the 2-D round trip unit-gain
//! live in the 2-D process (AV1 §7.13.3), implemented separately.

use crate::butterfly::{b, brev, cos128, h};
use crate::math::round2;

/// In-place 1-D inverse DCT of the length-`2^n` array `t` (`2 ≤ n ≤ 6`), per AV1 §7.13.2.3.
///
/// `r` is the intermediate Hadamard clamping range in bits (the caller passes `rowClampRange` /
/// `colClampRange` from the 2-D process). Only the first `1 << n` entries of `t` are touched.
///
/// # Panics
/// Panics if `t.len() < (1 << n)` or `n` is outside `2..=6`.
pub fn inverse_dct(t: &mut [i64], n: u32, r: u32) {
    assert!((2..=6).contains(&n), "inverse_dct: n must be in 2..=6");
    let len = 1usize << n;
    assert!(t.len() >= len, "inverse_dct: array shorter than 2^n");
    let n = n as i32;

    permute(t, n);

    // The ordered steps of AV1 §7.13.2.3. Each `for i = 0..K` in the spec is inclusive of `K`.
    if n == 6 {
        for i in 0..16 {
            b(t, 32 + i, 63 - i, 63 - 4 * brev(4, i), false, r);
        }
    }
    if n >= 5 {
        for i in 0..8 {
            b(t, 16 + i, 31 - i, 6 + (brev(3, 7 - i) << 3), false, r);
        }
    }
    if n == 6 {
        for i in 0..16 {
            h(t, 32 + i * 2, 33 + i * 2, (i & 1) != 0, r);
        }
    }
    if n >= 4 {
        for i in 0..4 {
            b(t, 8 + i, 15 - i, 12 + (brev(2, 3 - i) << 4), false, r);
        }
    }
    if n >= 5 {
        for i in 0..8 {
            h(t, 16 + 2 * i, 17 + 2 * i, (i & 1) != 0, r);
        }
    }
    if n == 6 {
        for i in 0..4 {
            for j in 0..2 {
                b(
                    t,
                    62 - i * 4 - j,
                    33 + i * 4 + j,
                    60 - 16 * brev(2, i) + 64 * j,
                    true,
                    r,
                );
            }
        }
    }
    if n >= 3 {
        for i in 0..2 {
            b(t, 4 + i, 7 - i, 56 - 32 * i, false, r);
        }
    }
    if n >= 4 {
        for i in 0..4 {
            h(t, 8 + 2 * i, 9 + 2 * i, (i & 1) != 0, r);
        }
    }
    if n >= 5 {
        for i in 0..2 {
            for j in 0..2 {
                b(
                    t,
                    30 - 4 * i - j,
                    17 + 4 * i + j,
                    24 + (j << 6) + ((1 - i) << 5),
                    true,
                    r,
                );
            }
        }
    }
    if n == 6 {
        for i in 0..8 {
            for j in 0..2 {
                h(t, 32 + i * 4 + j, 35 + i * 4 - j, (i & 1) != 0, r);
            }
        }
    }
    for i in 0..2 {
        b(t, 2 * i, 2 * i + 1, 32 + 16 * i, (1 - i) != 0, r);
    }
    if n >= 3 {
        for i in 0..2 {
            h(t, 4 + 2 * i, 5 + 2 * i, i != 0, r);
        }
    }
    if n >= 4 {
        for i in 0..2 {
            b(t, 14 - i, 9 + i, 48 + 64 * i, true, r);
        }
    }
    if n >= 5 {
        for i in 0..4 {
            for j in 0..2 {
                h(t, 16 + 4 * i + j, 19 + 4 * i - j, (i & 1) != 0, r);
            }
        }
    }
    if n == 6 {
        for i in 0..2 {
            for j in 0..4 {
                b(
                    t,
                    61 - i * 8 - j,
                    34 + i * 8 + j,
                    56 - i * 32 + (j >> 1) * 64,
                    true,
                    r,
                );
            }
        }
    }
    for i in 0..2 {
        h(t, i, 3 - i, false, r);
    }
    if n >= 3 {
        b(t, 6, 5, 32, true, r);
    }
    if n >= 4 {
        for i in 0..2 {
            for j in 0..2 {
                h(t, 8 + 4 * i + j, 11 + 4 * i - j, i != 0, r);
            }
        }
    }
    if n >= 5 {
        for i in 0..4 {
            b(t, 29 - i, 18 + i, 48 + (i >> 1) * 64, true, r);
        }
    }
    if n == 6 {
        for i in 0..4 {
            for j in 0..4 {
                h(t, 32 + 8 * i + j, 39 + 8 * i - j, (i & 1) != 0, r);
            }
        }
    }
    if n >= 3 {
        for i in 0..4 {
            h(t, i, 7 - i, false, r);
        }
    }
    if n >= 4 {
        for i in 0..2 {
            b(t, 13 - i, 10 + i, 32, true, r);
        }
    }
    if n >= 5 {
        for i in 0..2 {
            for j in 0..4 {
                h(t, 16 + i * 8 + j, 23 + i * 8 - j, i != 0, r);
            }
        }
    }
    if n == 6 {
        for i in 0..8 {
            b(t, 59 - i, 36 + i, if i < 4 { 48 } else { 112 }, true, r);
        }
    }
    if n >= 4 {
        for i in 0..8 {
            h(t, i, 15 - i, false, r);
        }
    }
    if n >= 5 {
        for i in 0..4 {
            b(t, 27 - i, 20 + i, 32, true, r);
        }
    }
    if n == 6 {
        for i in 0..8 {
            h(t, 32 + i, 47 - i, false, r);
            h(t, 48 + i, 63 - i, true, r);
        }
    }
    if n >= 5 {
        for i in 0..16 {
            h(t, i, 31 - i, false, r);
        }
    }
    if n == 6 {
        for i in 0..8 {
            b(t, 55 - i, 40 + i, 32, true, r);
        }
    }
    if n == 6 {
        for i in 0..32 {
            h(t, i, 63 - i, false, r);
        }
    }
}

/// `Inverse DCT array permutation` (AV1 §7.13.2.2): `T[i] = copyT[brev(n, i)]`.
fn permute(t: &mut [i64], n: i32) {
    let len = 1usize << n;
    let mut copy = [0i64; 64];
    copy[..len].copy_from_slice(&t[..len]);
    for (i, slot) in t[..len].iter_mut().enumerate() {
        *slot = copy[brev(n as u32, i as i32) as usize];
    }
}

/// In-place 1-D forward DCT-II of the length-`2^n` array `t` (`2 ≤ n ≤ 6`).
///
/// The *orthonormal* DCT-II — the true inverse pair of [`inverse_dct`] (whose `angle = 32`
/// butterfly bakes in the same `1/√2` DC weighting). `X[k] = w_k · Σ_m t[m] · cos(π(2m+1)k /
/// 2^{n+1})` with `w_0 = 1/√2` and `w_k = 1`, evaluated on the exact integer `cos128` table so the
/// transform is deterministic (no floating point). Pairing the orthonormal forward with the
/// orthonormal inverse means the only round-trip loss is quantization, not transform mismatch; the
/// absolute scale is reconciled with the per-pass shifts in the 2-D process (AV1 §7.13.3).
///
/// # Panics
/// Panics if `t.len() < (1 << n)` or `n` is outside `2..=6`.
pub fn forward_dct(t: &mut [i64], n: u32) {
    assert!((2..=6).contains(&n), "forward_dct: n must be in 2..=6");
    let len = 1usize << n;
    assert!(t.len() >= len, "forward_dct: array shorter than 2^n");

    // angle(m, k) = (64 / N) · (2m+1) · k, in units of pi/128, always an integer for N = 2^n ≤ 64.
    let step = 64i32 >> n;
    let mut out = [0i64; 64];
    for (k, slot) in out[..len].iter_mut().enumerate() {
        let mut acc = 0i64;
        for (m, &x) in t[..len].iter().enumerate() {
            let angle = step * (2 * m as i32 + 1) * k as i32;
            acc += x * cos128(angle);
        }
        // AC drops the cos128 `4096` scale (`>> 12`); DC additionally folds in `1/√2` via the
        // `2896/4096` cosine constant (`* 2896 >> 24`), matching the inverse's DC weighting.
        *slot = if k == 0 {
            round2(acc * 2896, 24)
        } else {
            round2(acc, 12)
        };
    }
    t[..len].copy_from_slice(&out[..len]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// Small deterministic LCG (reproducible, no `rand` dependency), matching `wht.rs`.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0
        }
        /// Coefficient in [-range, range].
        fn coeff(&mut self, range: i64) -> i64 {
            (self.next() >> 33) as i64 % (2 * range + 1) - range
        }
    }

    /// Orthonormal-relative DCT weight: the DC (k = 0) basis carries `1/√2` versus the AC bases.
    fn dct_weight(k: usize) -> f64 {
        if k == 0 {
            std::f64::consts::FRAC_1_SQRT_2
        } else {
            1.0
        }
    }

    /// Naive orthonormal DCT-III (the exact inverse of the orthonormal DCT-II up to a global
    /// per-size scale): the independent oracle for [`inverse_dct`]. `out[m] = Σ_k w_k · x[k] ·
    /// cos(π(2m+1)k / 2N)`. The AV1 inverse equals this up to a constant scale, compared
    /// proportionally.
    fn naive_idct(x: &[f64]) -> Vec<f64> {
        let n = x.len();
        (0..n)
            .map(|m| {
                (0..n)
                    .map(|k| {
                        dct_weight(k)
                            * x[k]
                            * (PI * (2 * m + 1) as f64 * k as f64 / (2.0 * n as f64)).cos()
                    })
                    .sum()
            })
            .collect()
    }

    /// Naive orthonormal DCT-II: the oracle for [`forward_dct`]. `X[k] = w_k · Σ_m x[m] ·
    /// cos(π(2m+1)k / 2N)`.
    fn naive_dct(x: &[f64]) -> Vec<f64> {
        let n = x.len();
        (0..n)
            .map(|k| {
                dct_weight(k)
                    * (0..n)
                        .map(|m| {
                            x[m] * (PI * (2 * m + 1) as f64 * k as f64 / (2.0 * n as f64)).cos()
                        })
                        .sum::<f64>()
            })
            .collect()
    }

    /// Assert `got` is proportional to `want` (same linear map up to one global scale), tolerant of
    /// the fixed-point rounding in the butterfly. Returns the measured scale.
    fn assert_proportional(got: &[i64], want: &[f64], tol: f64, ctx: &str) -> f64 {
        // Anchor the scale on the largest-magnitude reference entry.
        let (anchor, &wmax) = want
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().partial_cmp(&b.1.abs()).unwrap())
            .unwrap();
        assert!(wmax.abs() > 1e-6, "{ctx}: degenerate reference");
        let scale = got[anchor] as f64 / wmax;
        for (m, (&g, &w)) in got.iter().zip(want).enumerate() {
            let predicted = scale * w;
            assert!(
                (g as f64 - predicted).abs() <= tol,
                "{ctx}: entry {m} = {g}, expected ≈ {predicted:.2} (scale {scale:.3})",
            );
        }
        scale
    }

    #[test]
    fn inverse_dct_matches_naive_idct() {
        let mut rng = Lcg(0x1234_5678_9abc_def0);
        for n in 2..=6u32 {
            let len = 1usize << n;
            for _ in 0..200 {
                // Moderate inputs keep the r=16 Hadamard clamp inactive, so the map is linear.
                let coeffs: Vec<i64> = (0..len).map(|_| rng.coeff(64)).collect();
                let mut t = coeffs.clone();
                inverse_dct(&mut t, n, 16);
                let want = naive_idct(&coeffs.iter().map(|&c| c as f64).collect::<Vec<_>>());
                // Tolerance scales with size: more butterfly stages accumulate more rounding.
                assert_proportional(&t[..len], &want, 2.0 * len as f64, &format!("idct n={n}"));
            }
        }
    }

    #[test]
    fn forward_dct_matches_naive_dct() {
        let mut rng = Lcg(0x0fed_cba9_8765_4321);
        for n in 2..=6u32 {
            let len = 1usize << n;
            for _ in 0..200 {
                let resid: Vec<i64> = (0..len).map(|_| rng.coeff(255)).collect();
                let mut t = resid.clone();
                forward_dct(&mut t, n);
                let want = naive_dct(&resid.iter().map(|&c| c as f64).collect::<Vec<_>>());
                assert_proportional(&t[..len], &want, len as f64, &format!("dct n={n}"));
            }
        }
    }

    #[test]
    fn forward_then_inverse_is_proportional_identity() {
        // A DCT-II followed by the DCT-III inverse is a scaled identity; with both transforms
        // correct, inverse_dct(forward_dct(x)) must be proportional to x.
        let mut rng = Lcg(0xdead_beef_cafe_0001);
        for n in 2..=6u32 {
            let len = 1usize << n;
            for _ in 0..100 {
                let resid: Vec<i64> = (0..len).map(|_| rng.coeff(120)).collect();
                let mut t = resid.clone();
                forward_dct(&mut t, n);
                inverse_dct(&mut t, n, 24);
                let want: Vec<f64> = resid.iter().map(|&c| c as f64).collect();
                assert_proportional(&t[..len], &want, 6.0 * len as f64, &format!("rt n={n}"));
            }
        }
    }

    #[test]
    fn dc_input_is_flat_after_inverse() {
        // A single DC coefficient inverse-transforms to a constant block.
        for n in 2..=6u32 {
            let len = 1usize << n;
            let mut t = vec![0i64; len];
            t[0] = 100;
            inverse_dct(&mut t, n, 16);
            let first = t[0];
            for (m, &v) in t[..len].iter().enumerate() {
                assert_eq!(v, first, "n={n}: DC should be flat, entry {m} differs");
            }
        }
    }

    #[test]
    #[should_panic(expected = "array shorter than 2^n")]
    fn inverse_dct_panics_on_short_array() {
        // The length precondition is `t.len() >= 1 << n`. A mutated `1 << n` (e.g. `1 >> n == 0`)
        // makes the assert vacuous; pin it via the exact panic message (a later out-of-bounds panic
        // would carry a different one).
        let mut t = [0i64; 3];
        inverse_dct(&mut t, 2, 16);
    }

    #[test]
    fn forward_dct_exact_values() {
        // The proportional test pins forward_dct only up to a global scale, so a negated (`acc -=`),
        // zeroed (`acc *=`), or `x * cos -> x + cos` accumulation survives it. These exact snapshots
        // (the implementation is independently shown correct by the proportional naive-DCT test) pin
        // the actual values, sign and magnitude included.
        let mut t = [100i64, 100, 100, 100];
        forward_dct(&mut t, 2);
        assert_eq!(t, [283, 0, 0, 0]); // flat input -> DC only
        let mut t = [200i64, -100, 50, -25, 10, 0, 5, -5];
        forward_dct(&mut t, 3);
        assert_eq!(t, [95, 135, 139, 161, 159, 198, 214, 174]);
    }
}
