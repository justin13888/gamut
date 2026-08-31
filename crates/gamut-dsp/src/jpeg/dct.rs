//! JPEG-1 8×8 forward and inverse discrete cosine transforms (ITU-T T.81 | ISO/IEC 10918-1
//! §A.3.3).
//!
//! [`fdct8x8`] and [`idct8x8`] are direct transcriptions of the *informative* ideal FDCT / IDCT
//! equations of T.81 §A.3.3:
//!
//! ```text
//! FDCT:  S_vu = 1/4 · C_u C_v · Σ_{x=0}^{7} Σ_{y=0}^{7} s_yx · cos((2x+1)uπ/16) · cos((2y+1)vπ/16)
//! IDCT:  s_yx = 1/4 · Σ_{u=0}^{7} Σ_{v=0}^{7} C_u C_v · S_vu · cos((2x+1)uπ/16) · cos((2y+1)vπ/16)
//! with   C_u, C_v = 1/√2 for u, v = 0, and 1 otherwise.
//! ```
//!
//! §A.3.3 flags these as *ideal* functional definitions that "cannot be represented with perfect
//! accuracy by any real implementation"; the normative accuracy requirements for a conforming
//! DCT live in T.83 (ISO/IEC 10918-2). This module targets clarity and spec-exactness rather than
//! speed — it evaluates the separable transform in `f64` and rounds once at the end. Encoder
//! performance tuning (fixed-point / fast-DCT kernels) is deliberately out of scope for issue #28.
//!
//! Both transforms are **separable**: the 2-D kernel is the product of two 1-D cosine kernels, so
//! `fdct8x8` transforms each row (spatial index `x` → frequency `u`) then each column (`y` → `v`),
//! and `idct8x8` inverts column-then-row. The row/column pass order does not affect the real-valued
//! result; the single rounding is applied only when writing the integer output.
//!
//! Sample orientation follows T.81 Figure A.4: `s_yx` is the sample at row `y`, column `x`, and
//! both blocks are stored in **raster (row-major, natural) order** — element `y·8 + x` for samples
//! and `v·8 + u` for coefficients. This is the *natural* order of §A.3.6, not the zig-zag order.

use std::f64::consts::{FRAC_1_SQRT_2, PI};

/// The T.81 §A.3.3 normalization scale `C_k`: `1/√2` at the DC index `k = 0`, and `1` for every AC
/// index `k = 1..=7`.
fn normalization(k: usize) -> f64 {
    if k == 0 { FRAC_1_SQRT_2 } else { 1.0 }
}

/// The DCT cosine basis `cos((2·i + 1)·k·π / 16)` for spatial index `i` and frequency index `k`,
/// both spanning `0..8`. Recomputed per call from [`f64::cos`] (deterministic; no allocation, no
/// precomputed table to drift out of sync with the spec).
fn cosine_table() -> [[f64; 8]; 8] {
    let mut table = [[0.0f64; 8]; 8];
    for (i, row) in table.iter_mut().enumerate() {
        for (k, cell) in row.iter_mut().enumerate() {
            *cell = (((2 * i + 1) * k) as f64 * PI / 16.0).cos();
        }
    }
    table
}

/// One 1-D 8-point forward DCT-II with the T.81 §A.3.3 normalization: `F_k = ½·C_k·Σ_i f_i·
/// cos((2i+1)kπ/16)`. Two of these (over `x` then `y`) compose the separable 2-D FDCT.
fn forward_1d(input: &[f64; 8], cos: &[[f64; 8]; 8]) -> [f64; 8] {
    let mut out = [0.0f64; 8];
    for (k, out_k) in out.iter_mut().enumerate() {
        let mut sum = 0.0;
        for (i, &sample) in input.iter().enumerate() {
            sum += sample * cos[i][k];
        }
        *out_k = 0.5 * normalization(k) * sum;
    }
    out
}

/// One 1-D 8-point inverse DCT-III with the T.81 §A.3.3 normalization: `f_i = ½·Σ_k C_k·F_k·
/// cos((2i+1)kπ/16)`. Two of these (over `v` then `u`) compose the separable 2-D IDCT.
fn inverse_1d(input: &[f64; 8], cos: &[[f64; 8]; 8]) -> [f64; 8] {
    let mut out = [0.0f64; 8];
    for (i, out_i) in out.iter_mut().enumerate() {
        let mut sum = 0.0;
        for (k, &coeff) in input.iter().enumerate() {
            sum += normalization(k) * coeff * cos[i][k];
        }
        *out_i = 0.5 * sum;
    }
    out
}

/// Rounds a transform result to the nearest integer, ties **away from zero** ([`f64::round`]).
///
/// The same rule is used by [`fdct8x8`] and [`idct8x8`] so the two directions are consistent. The
/// `as i32` cast saturates on out-of-range inputs (Rust's defined float→int behaviour), but a valid
/// 8-/12-bit block never approaches `i32::MAX`.
fn round_to_i32(value: f64) -> i32 {
    value.round() as i32
}

/// In-place 2-D **forward** DCT of an 8×8 block, per ITU-T T.81 | ISO/IEC 10918-1 §A.3.3.
///
/// `block` holds the 64 source samples in raster (row-major / natural) order — element `y·8 + x`
/// is the sample `s_yx` at row `y`, column `x` (T.81 Figure A.4). On return `block` holds the
/// unquantized DCT coefficients, likewise in raster order (`v·8 + u` is `S_vu`), each rounded to
/// the nearest integer with ties away from zero.
///
/// # Input domain
/// The samples must already be **level-shifted** to a signed representation per §A.3.1 (subtract
/// `2^(P-1)`): for 8-bit precision (`P = 8`) the domain is `-128..=127`; for 12-bit (`P = 12`) it
/// is `-2048..=2047`. Level shifting is the caller's responsibility — this kernel is pure DCT.
///
/// # Output range
/// The DC term `S_00` is `8×` the block mean, so `|coeff| ≤ 8 · max|sample|`: at most `±1024` for
/// 8-bit input and `±16384` for 12-bit input; every AC term is smaller in magnitude.
///
/// # Examples
/// ```
/// use gamut_dsp::jpeg::fdct8x8;
///
/// // A flat (constant) block collapses to a pure DC coefficient: S_00 = 8·c, all AC = 0.
/// let mut block = [10i32; 64];
/// fdct8x8(&mut block);
/// assert_eq!(block[0], 80);
/// assert!(block[1..].iter().all(|&c| c == 0));
/// ```
pub fn fdct8x8(block: &mut [i32; 64]) {
    let cos = cosine_table();
    let mut work = [0.0f64; 64];

    // Row pass: 1-D forward DCT along x within each row y → intermediate T[y][u].
    for y in 0..8 {
        let mut row = [0.0f64; 8];
        for (x, cell) in row.iter_mut().enumerate() {
            *cell = f64::from(block[y * 8 + x]);
        }
        let transformed = forward_1d(&row, &cos);
        for (u, &value) in transformed.iter().enumerate() {
            work[y * 8 + u] = value;
        }
    }

    // Column pass: 1-D forward DCT along y for each frequency column u → S[v][u].
    for u in 0..8 {
        let mut col = [0.0f64; 8];
        for (y, cell) in col.iter_mut().enumerate() {
            *cell = work[y * 8 + u];
        }
        let transformed = forward_1d(&col, &cos);
        for (v, &value) in transformed.iter().enumerate() {
            block[v * 8 + u] = round_to_i32(value);
        }
    }
}

/// In-place 2-D **inverse** DCT of an 8×8 block, per ITU-T T.81 | ISO/IEC 10918-1 §A.3.3.
///
/// `block` holds the 64 dequantized DCT coefficients in raster (row-major) order — element
/// `v·8 + u` is `S_vu`. On return `block` holds the reconstructed samples in raster order
/// (`y·8 + x` is `s_yx`), each rounded to the nearest integer with ties away from zero — the same
/// rounding rule as [`fdct8x8`].
///
/// # Output
/// The result is the **level-shifted** reconstruction: it is *not* clamped and the `2^(P-1)` level
/// shift is *not* added back. Restoring the unsigned sample range (add `2^(P-1)`, clamp to
/// `0..=2^P − 1`) is the decoder's job per §A.3.1, kept out of this pure kernel.
///
/// # Examples
/// ```
/// use gamut_dsp::jpeg::idct8x8;
///
/// // A pure DC coefficient reconstructs to a flat block: s_yx = S_00 / 8 everywhere.
/// let mut block = [0i32; 64];
/// block[0] = 80;
/// idct8x8(&mut block);
/// assert!(block.iter().all(|&s| s == 10));
/// ```
pub fn idct8x8(block: &mut [i32; 64]) {
    let cos = cosine_table();
    let mut work = [0.0f64; 64];

    // Column pass: 1-D inverse DCT along v for each frequency column u → intermediate P[y][u].
    for u in 0..8 {
        let mut col = [0.0f64; 8];
        for (v, cell) in col.iter_mut().enumerate() {
            *cell = f64::from(block[v * 8 + u]);
        }
        let transformed = inverse_1d(&col, &cos);
        for (y, &value) in transformed.iter().enumerate() {
            work[y * 8 + u] = value;
        }
    }

    // Row pass: 1-D inverse DCT along u within each row y → reconstructed s[y][x].
    for y in 0..8 {
        let mut row = [0.0f64; 8];
        for (u, cell) in row.iter_mut().enumerate() {
            *cell = work[y * 8 + u];
        }
        let transformed = inverse_1d(&row, &cos);
        for (x, &value) in transformed.iter().enumerate() {
            block[y * 8 + x] = round_to_i32(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testrng::Lcg;

    /// Largest tolerated gap between an integer transform output and the ideal real-valued §A.3.3
    /// result. A correct nearest-integer rounding is within `0.5`; the `1e-6` slack absorbs the
    /// ~`1e-9` difference between the production kernel's separable summation order and the oracle's
    /// direct double sum (which only ever matters when the true value sits within an ULP of a `x.5`
    /// tie). Any mutation to an index, loop bound, or constant moves an output by ≫ 1 and trips this.
    const ROUND_TOL: f64 = 0.5 + 1e-6;

    /// Independent, self-contained O(64²) evaluation of the ideal (unrounded) §A.3.3 FDCT equation
    /// (double sum over `x, y`). Deliberately *not* separable — the oracle the production kernel's
    /// separable evaluation must round to.
    fn direct_fdct(block: &[i32; 64]) -> [f64; 64] {
        let mut out = [0.0f64; 64];
        for v in 0..8usize {
            for u in 0..8usize {
                let mut sum = 0.0f64;
                for x in 0..8usize {
                    for y in 0..8usize {
                        sum += f64::from(block[y * 8 + x])
                            * (((2 * x + 1) * u) as f64 * PI / 16.0).cos()
                            * (((2 * y + 1) * v) as f64 * PI / 16.0).cos();
                    }
                }
                out[v * 8 + u] = 0.25 * normalization(u) * normalization(v) * sum;
            }
        }
        out
    }

    /// Independent O(64²) evaluation of the ideal (unrounded) §A.3.3 IDCT equation, oracle for
    /// [`idct8x8`].
    fn direct_idct(block: &[i32; 64]) -> [f64; 64] {
        let mut out = [0.0f64; 64];
        for y in 0..8usize {
            for x in 0..8usize {
                let mut sum = 0.0f64;
                for u in 0..8usize {
                    for v in 0..8usize {
                        sum += normalization(u)
                            * normalization(v)
                            * f64::from(block[v * 8 + u])
                            * (((2 * x + 1) * u) as f64 * PI / 16.0).cos()
                            * (((2 * y + 1) * v) as f64 * PI / 16.0).cos();
                    }
                }
                out[y * 8 + x] = 0.25 * sum;
            }
        }
        out
    }

    /// Asserts every element of `got` is the correct nearest-integer rounding of `ideal`.
    fn assert_rounds(got: &[i32; 64], ideal: &[f64; 64], ctx: &str) {
        for (idx, (&g, &t)) in got.iter().zip(ideal.iter()).enumerate() {
            assert!(
                (f64::from(g) - t).abs() <= ROUND_TOL,
                "{ctx}: index {idx} got {g}, ideal {t}",
            );
        }
    }

    /// The workhorse: over a battery of deterministic pseudo-random blocks spanning the full 8-bit
    /// and 12-bit level-shifted domains, the separable [`fdct8x8`] must round the independent direct
    /// §A.3.3 double sum to the nearest integer. This pins index orientation (x/y ↔ u/v), the
    /// `¼·C_u·C_v` scale, and every cosine argument — a swapped index, off-by-one, or wrong constant
    /// diverges by far more than half a unit.
    #[test]
    fn fdct_matches_direct_formula() {
        let mut rng = Lcg::new(0x1234_5678_9abc_def0);
        for _ in 0..600 {
            let mut block = [0i32; 64];
            for s in &mut block {
                *s = rng.level_shifted_sample(8);
            }
            let mut got = block;
            fdct8x8(&mut got);
            assert_rounds(&got, &direct_fdct(&block), "fdct 8-bit");
        }
        for _ in 0..200 {
            let mut block = [0i32; 64];
            for s in &mut block {
                *s = rng.level_shifted_sample(12);
            }
            let mut got = block;
            fdct8x8(&mut got);
            assert_rounds(&got, &direct_fdct(&block), "fdct 12-bit");
        }
    }

    /// Same battery for the inverse: [`idct8x8`] must round the independent direct double sum.
    #[test]
    fn idct_matches_direct_formula() {
        let mut rng = Lcg::new(0x0fed_cba9_8765_4321);
        for _ in 0..600 {
            let mut coeffs = [0i32; 64];
            for c in &mut coeffs {
                *c = rng.level_shifted_sample(8);
            }
            let mut got = coeffs;
            idct8x8(&mut got);
            assert_rounds(&got, &direct_idct(&coeffs), "idct 8-bit");
        }
        for _ in 0..200 {
            let mut coeffs = [0i32; 64];
            for c in &mut coeffs {
                *c = rng.level_shifted_sample(12);
            }
            let mut got = coeffs;
            idct8x8(&mut got);
            assert_rounds(&got, &direct_idct(&coeffs), "idct 12-bit");
        }
    }

    /// Pins the **sign** of the private 1-D kernels. The 2-D transforms compose each 1-D kernel
    /// twice (rows then columns), so a sign flip inside `forward_1d`/`inverse_1d` cancels itself
    /// and is invisible to every 2-D test — only a direct 1-D check can catch it. A constant input
    /// `c` must give a *positive* DC term `8·c·½·(1/√2) = 2√2·c` (and the inverse must map that DC
    /// back to the positive constant), so a negated accumulator trips the sign assertion here.
    #[test]
    fn one_dimensional_kernels_have_positive_dc_sign() {
        let cos = cosine_table();
        let constant = [3.0f64; 8];
        let spectrum = forward_1d(&constant, &cos);
        let expected_dc = 8.0 * 3.0 * 0.5 * FRAC_1_SQRT_2;
        assert!(
            (spectrum[0] - expected_dc).abs() < 1e-12,
            "forward_1d DC: got {}, expected {expected_dc}",
            spectrum[0]
        );
        for (k, &ac) in spectrum.iter().enumerate().skip(1) {
            assert!(ac.abs() < 1e-12, "forward_1d AC[{k}] = {ac}, expected 0");
        }

        let mut dc_only = [0.0f64; 8];
        dc_only[0] = expected_dc;
        let reconstructed = inverse_1d(&dc_only, &cos);
        for (i, &s) in reconstructed.iter().enumerate() {
            assert!(
                (s - 3.0).abs() < 1e-12,
                "inverse_1d sample[{i}] = {s}, expected 3"
            );
        }
    }

    /// A constant block `c` is pure DC: `S_00 = 8·c` exactly, all 63 AC coefficients exactly zero.
    /// This pins the DC normalization (`C_0 = 1/√2`, the `¼` scale) and cosine orthogonality.
    #[test]
    fn fdct_constant_block_is_dc() {
        for c in [-128i32, -1, 0, 1, 42, 127] {
            let mut block = [c; 64];
            fdct8x8(&mut block);
            assert_eq!(block[0], 8 * c, "DC for constant {c}");
            assert!(block[1..].iter().all(|&x| x == 0), "AC not zero for {c}");
        }
    }

    /// The all-zero block maps to all zeros under both transforms.
    #[test]
    fn zero_block_is_fixed_point() {
        let mut block = [0i32; 64];
        fdct8x8(&mut block);
        assert_eq!(block, [0i32; 64]);
        idct8x8(&mut block);
        assert_eq!(block, [0i32; 64]);
    }

    /// A single DC coefficient `k` reconstructs to the flat block `k/8` under the IDCT — the inverse
    /// of `fdct_constant_block_is_dc`, pinning the inverse DC normalization and `¼` scale.
    #[test]
    fn idct_dc_only_is_constant() {
        for k in [-1024i32, -8, 0, 8, 80, 1024] {
            let mut block = [0i32; 64];
            block[0] = k;
            idct8x8(&mut block);
            assert!(block.iter().all(|&s| s == k / 8), "flat value for DC {k}");
        }
    }

    /// Orientation guard: a **purely vertical** pattern `s_yx = g(y)` (constant along each row) has
    /// no horizontal frequency content, so the FDCT must leave *only* the `u = 0` coefficient column
    /// nonzero. Symmetrically, a **purely horizontal** pattern leaves only the `v = 0` row nonzero.
    /// These zeros are exact (cosine orthogonality against a constant), so they pin the `x ↔ u`
    /// (horizontal) / `y ↔ v` (vertical) mapping deterministically — an x/y index swap moves the
    /// nonzero energy to the wrong axis and trips this.
    #[test]
    fn fdct_preserves_axis_orientation() {
        let profile = [17i32, -40, 63, 5, -88, 100, -12, 34];

        // Vertical: value depends on row y only → energy confined to coefficient column u = 0.
        let mut block = [0i32; 64];
        for y in 0..8 {
            for x in 0..8 {
                block[y * 8 + x] = profile[y];
            }
        }
        fdct8x8(&mut block);
        for v in 0..8 {
            for u in 1..8 {
                assert_eq!(block[v * 8 + u], 0, "vertical: AC at (v={v}, u={u})");
            }
        }
        assert!((1..8).any(|v| block[v * 8] != 0), "vertical: no v-energy");

        // Horizontal: value depends on column x only → energy confined to coefficient row v = 0.
        let mut block = [0i32; 64];
        for y in 0..8 {
            for x in 0..8 {
                block[y * 8 + x] = profile[x];
            }
        }
        fdct8x8(&mut block);
        for v in 1..8 {
            for u in 0..8 {
                assert_eq!(block[v * 8 + u], 0, "horizontal: AC at (v={v}, u={u})");
            }
        }
        assert!((1..8).any(|u| block[u] != 0), "horizontal: no u-energy");
    }

    /// Forward then inverse recovers the original 8-bit level-shifted block within ±1 per sample.
    /// The tolerance is ±1 because the pipeline rounds twice independently — once to integer DCT
    /// coefficients, once to integer samples — and each rounding contributes at most half a unit;
    /// for these random blocks the combined per-sample error never exceeds one.
    #[test]
    fn forward_inverse_round_trip() {
        let mut rng = Lcg::new(0xdead_beef_cafe_0007);
        for _ in 0..400 {
            let mut original = [0i32; 64];
            for s in &mut original {
                *s = rng.level_shifted_sample(8);
            }
            let mut block = original;
            fdct8x8(&mut block);
            idct8x8(&mut block);
            for (idx, (&r, &o)) in block.iter().zip(original.iter()).enumerate() {
                assert!(
                    (r - o).abs() <= 1,
                    "round-trip drift {} at index {idx} (orig {o}, recon {r})",
                    r - o
                );
            }
        }
    }
}
