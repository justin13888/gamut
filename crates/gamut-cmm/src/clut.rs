//! Multi-dimensional CLUT interpolation: [`ClutTable`], the evaluation form of a parsed
//! [`gamut_icc::Clut`], applied per pixel by [`Stage::Clut`](crate::Stage::Clut).
//!
//! ICC.1:2022 §10.12.3 defines the CLUT *data layout* (a regular grid of output samples, last
//! input channel varying fastest, output channels interleaved per node) but not how a CMM
//! interpolates between nodes. Interpolation semantics therefore follow the oracle, lcms2
//! (`src/cmsintrp.c`, transcribed in `references/cmm/README.md`):
//!
//! - **Input mapping** — each input is clamped by lcms2's `fclamp` (NaN and everything below
//!   `1e-9` → `0.0`, above `1.0` → `1.0`), scaled by `grid_points − 1`, and split into a floor
//!   cell index and a fraction. The upper neighbour index equals the lower one when the clamped
//!   input is exactly `1.0` (lcms2's edge rule, which keeps the last node in bounds).
//! - **[`ClutInterpolation::Tetrahedral`]** (default for ≥ 3 inputs, matching lcms2's
//!   interpolator selection) — the 3-D case is the exact six-branch Sakamoto/Kasson
//!   decomposition of `TetrahedralInterpFloat`, `>=` comparisons in lcms2's order so ties
//!   resolve identically; ≥ 4 inputs recurse lcms2's `Eval4InputsFloat`…`Eval15InputsFloat`
//!   scheme — slice the outermost axis, evaluate the two inner sub-grids, blend linearly —
//!   bottoming out in the 3-D tetrahedral base.
//! - **[`ClutInterpolation::Multilinear`]** (default and only mode for 1–2 inputs; selectable
//!   for ≥ 3) — classic 2ᴺ-corner multilinear: linear in 1-D, bilinear in 2-D, trilinear in
//!   3-D (lcms2's `LERP` X-then-Y-then-Z order), and the same outermost-axis slice-and-blend
//!   recursion above 3-D (which is algebraically the multilinear form). lcms2 reaches its
//!   trilinear path only when a CLUT is Lab-indexed (B2A/devicelink with Lab PCS,
//!   `ChangeInterpolationToTrilinear` in `cmsio1.c`); that selection happens at profile-link
//!   time (#328), which is why the mode is carried per table via
//!   [`ClutTable::with_interpolation`].
//!
//! One deliberate divergence, documented on [`ClutTable::new`]: axes with a **single grid
//! node** interpolate as constant along that axis. lcms2's 2-D/3-D/N-D float routines have no
//! `Domain == 0` guard and read one node past the end for such an axis; this crate pins the
//! sane semantics instead of the out-of-bounds read.

use crate::error::{CmmError, Result};
use crate::pipeline::MAX_CHANNELS;

/// The largest CLUT input dimension count, from lcms2's `MAX_INPUT_DIMENSIONS`
/// (`include/lcms2_plugin.h`): ICC device spaces stop at 15 colorants (`FCLR`), and the oracle
/// rejects wider grids, so behaviour above 15 inputs would be untestable against it.
const MAX_INPUT_DIMENSIONS: usize = 15;

/// lcms2's `fclamp` (`cmsintrp.c`): NaN, negatives, and sub-`1e-9` positives collapse to
/// exactly `0.0`; values above `1.0` clamp to `1.0` (spelt `min`, equivalent to lcms2's
/// `v > 1.0 ? 1.0 : v` since NaN is already gone).
fn fclamp(v: f64) -> f64 {
    if v.is_nan() || v < 1.0e-9 {
        0.0
    } else {
        v.min(1.0)
    }
}

/// `l + (h − l)·a` — lcms2's `LERP` macro.
fn lerp(a: f64, l: f64, h: f64) -> f64 {
    l + (h - l) * a
}

/// Maps one input onto a grid axis of `nodes` points: `(lower node, upper node, fraction)`.
///
/// The lcms2 float-path mapping: `px = fclamp(v) · (nodes − 1)`, lower node `⌊px⌋`, fraction
/// `px − ⌊px⌋`; the upper node equals the lower when the clamped input is exactly `1.0`
/// (lcms2's `>= 1.0` edge rule) — and, this crate's guard, when the axis has a single node
/// (`Domain == 0`, where lcms2 would index out of bounds; the fraction is `0` there, so the
/// axis interpolates as constant).
fn axis(v: f64, nodes: u8) -> (usize, usize, f64) {
    let clamped = fclamp(v);
    let domain = usize::from(nodes) - 1;
    let px = clamped * domain as f64;
    let floor = px.floor();
    let fraction = px - floor;
    let lower = floor as usize;
    let upper = if domain == 0 || clamped >= 1.0 {
        lower
    } else {
        lower + 1
    };
    (lower, upper, fraction)
}

/// How a [`ClutTable`] interpolates between grid nodes.
///
/// The discriminants are permanent and append-only (workspace C-portability convention).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ClutInterpolation {
    /// Classic 2ᴺ-corner multilinear (linear/bilinear/trilinear/…) for any dimension count.
    ///
    /// lcms2 uses this for 3-D only when the CLUT is **Lab-indexed** (a B2A or devicelink
    /// table whose input space is the Lab PCS) — profile linking (#328) selects it through
    /// [`ClutTable::with_interpolation`].
    Multilinear = 0,
    /// lcms2's default for device CLUTs of ≥ 3 inputs: the six-branch tetrahedral
    /// decomposition in 3-D, reached through outermost-axis slice-and-blend recursion above
    /// 3-D. Requires at least 3 input channels.
    Tetrahedral = 1,
}

/// A validated, evaluation-ready colour lookup table over a parsed [`gamut_icc::Clut`].
///
/// Construction ([`ClutTable::new`]/[`ClutTable::with_interpolation`]) is the validity
/// boundary: geometry (input dimensions `1..=15`, output channels `1..=16`, no zero-node axis,
/// the sample-count invariant) is checked once, and samples are normalized to `f64` in
/// `[0, 1]` by the table's full scale (`255` for 8-bit CLUT data, `65535` for 16-bit — the
/// parse widens 8-bit samples without rescaling, so the divisor must follow the precision).
/// Evaluation is then infallible and allocation-free (recursion above 3-D carries two
/// `[f64; 16]` stack buffers per sliced axis).
#[derive(Debug, Clone)]
pub struct ClutTable {
    /// Grid nodes per input axis, first axis slowest-varying; every entry ≥ 1.
    grid_points: Vec<u8>,
    /// Output samples per grid node, `1..=MAX_CHANNELS`.
    output_channels: u8,
    /// Node samples normalized to `[0, 1]`, in grid order (last input axis fastest, output
    /// channels interleaved per node — the §10.12.3 layout `gamut_icc::Clut` preserves).
    samples: Vec<f64>,
    /// The interpolation mode evaluation dispatches on.
    interpolation: ClutInterpolation,
}

/// The geometry invariants every [`ClutTable`] upholds, checked once per construction
/// (shared by the parsed and the computed-sample constructors): a non-empty input dimension
/// count within lcms2's `MAX_INPUT_DIMENSIONS`, an output channel count in
/// `1..=`[`MAX_CHANNELS`], no zero-node axis, `samples.len() == ∏ grid_points ×
/// output_channels`, and at least 3 input channels for tetrahedral interpolation.
fn check_geometry(
    grid_points: &[u8],
    output_channels: u8,
    sample_count: usize,
    interpolation: ClutInterpolation,
) -> Result<()> {
    let dims = grid_points.len();
    if dims == 0 {
        return Err(CmmError::ClutGeometry("no input dimensions"));
    }
    if dims > MAX_INPUT_DIMENSIONS {
        return Err(CmmError::TooManyChannels(
            u8::try_from(dims).unwrap_or(u8::MAX),
        ));
    }
    if output_channels == 0 || output_channels > MAX_CHANNELS {
        return Err(CmmError::TooManyChannels(output_channels));
    }
    if grid_points.contains(&0) {
        return Err(CmmError::ClutGeometry("zero grid axis"));
    }
    let nodes = grid_points
        .iter()
        .try_fold(1_usize, |acc, &n| acc.checked_mul(usize::from(n)));
    let expected = nodes.and_then(|n| n.checked_mul(usize::from(output_channels)));
    if expected != Some(sample_count) {
        return Err(CmmError::ClutGeometry("sample count mismatch"));
    }
    if interpolation == ClutInterpolation::Tetrahedral && dims < 3 {
        return Err(CmmError::ClutGeometry(
            "tetrahedral interpolation requires at least 3 input channels",
        ));
    }
    Ok(())
}

impl ClutTable {
    /// Builds an evaluation-ready table from a parsed CLUT with the lcms2 default
    /// interpolation: [`Tetrahedral`](ClutInterpolation::Tetrahedral) for 3 or more input
    /// channels, [`Multilinear`](ClutInterpolation::Multilinear) for 1–2 (where lcms2's
    /// linear/bilinear routines *are* the multilinear form).
    ///
    /// Lab-indexed CLUTs (B2A/devicelink tables entered from the Lab PCS) must instead be
    /// trilinear per lcms2 — profile linking (#328) requests that via
    /// [`ClutTable::with_interpolation`].
    ///
    /// One deliberate divergence from lcms2: an axis with a **single grid node** evaluates as
    /// constant along that axis. lcms2's multi-dimensional float routines lack a `Domain == 0`
    /// guard and read out of bounds for inputs below `1.0` on such an axis; this crate pins
    /// the in-bounds semantics.
    ///
    /// # Errors
    ///
    /// [`CmmError::TooManyChannels`] if the input dimension count (`grid_points.len()`)
    /// exceeds 15 (lcms2's `MAX_INPUT_DIMENSIONS`; ICC device spaces stop at 15 colorants) or
    /// the output channel count is outside `1..=`[`MAX_CHANNELS`];
    /// [`CmmError::ClutGeometry`] if there are no input dimensions, an axis has zero grid
    /// nodes, or `samples.len()` differs from `∏ grid_points × output_channels`.
    pub fn new(clut: &gamut_icc::Clut) -> Result<Self> {
        let mode = if clut.grid_points.len() >= 3 {
            ClutInterpolation::Tetrahedral
        } else {
            ClutInterpolation::Multilinear
        };
        Self::with_interpolation(clut, mode)
    }

    /// [`ClutTable::new`] with an explicit interpolation mode — the hook profile linking
    /// (#328) uses to force [`Multilinear`](ClutInterpolation::Multilinear) (trilinear) for
    /// Lab-indexed CLUTs, mirroring lcms2's `ChangeInterpolationToTrilinear`.
    ///
    /// # Errors
    ///
    /// Everything [`ClutTable::new`] reports, plus [`CmmError::ClutGeometry`] when
    /// [`Tetrahedral`](ClutInterpolation::Tetrahedral) is requested for fewer than 3 input
    /// channels (the decomposition is inherently 3-D; lcms2 never selects it below 3 inputs).
    pub fn with_interpolation(
        clut: &gamut_icc::Clut,
        interpolation: ClutInterpolation,
    ) -> Result<Self> {
        check_geometry(
            &clut.grid_points,
            clut.output_channels,
            clut.samples.len(),
            interpolation,
        )?;
        let full_scale = f64::from(clut.precision.full_scale());
        let samples = clut
            .samples
            .iter()
            .map(|&s| f64::from(s) / full_scale)
            .collect();
        Ok(Self {
            grid_points: clut.grid_points.clone(),
            output_channels: clut.output_channels,
            samples,
            interpolation,
        })
    }

    /// Builds a table directly from **computed** `f64` node samples, bypassing the parsed
    /// [`gamut_icc::Clut`] the public constructors require — the construction path
    /// [`crate::optimize`]'s CLUT resampling needs, where the nodes come from evaluating a
    /// pipeline rather than from a profile's quantized table.
    ///
    /// `samples` is in the same grid order as a parsed CLUT (last input axis fastest, output
    /// channels interleaved per node) and carries values already in the table's own domain —
    /// no full-scale division happens here. Geometry is validated exactly as
    /// [`ClutTable::with_interpolation`] validates it.
    ///
    /// # Errors
    ///
    /// Everything [`ClutTable::with_interpolation`] reports for an inconsistent geometry.
    #[must_use = "the constructed table is the only handle on the resampled grid"]
    pub(crate) fn from_samples(
        grid_points: Vec<u8>,
        output_channels: u8,
        samples: Vec<f64>,
        interpolation: ClutInterpolation,
    ) -> Result<Self> {
        check_geometry(&grid_points, output_channels, samples.len(), interpolation)?;
        Ok(Self {
            grid_points,
            output_channels,
            samples,
            interpolation,
        })
    }

    /// The number of input channels (grid dimensions), `1..=15`.
    #[must_use]
    pub fn input_channels(&self) -> u8 {
        // Bounded by MAX_INPUT_DIMENSIONS (15) at construction.
        self.grid_points.len() as u8
    }

    /// The number of output channels per node, `1..=`[`MAX_CHANNELS`].
    #[must_use]
    pub fn output_channels(&self) -> u8 {
        self.output_channels
    }

    /// Grid nodes per input axis, first axis slowest-varying (every entry ≥ 1) — the table's
    /// resolution, which fixes both its interpolation error and its memory. Read by
    /// [`crate::optimize`]'s resampling pass and by any caller sizing up a parsed CLUT.
    #[must_use]
    pub fn grid_points(&self) -> &[u8] {
        &self.grid_points
    }

    /// The interpolation mode this table evaluates with.
    #[must_use]
    pub fn interpolation(&self) -> ClutInterpolation {
        self.interpolation
    }

    /// Evaluates the table over one pixel: `input` holds `input_channels()` samples, `output`
    /// receives `output_channels()` samples. Lengths are guaranteed by the caller
    /// ([`Stage::eval`](crate::Stage), via `Pipeline`'s construction-time validation, or the
    /// unit tests).
    pub(crate) fn eval(&self, input: &[f64], output: &mut [f64]) {
        self.eval_grid(input, &self.grid_points, &self.samples, output);
    }

    /// Evaluates the sub-grid spanned by the trailing `axes` over `table`. Above 3 dimensions,
    /// slices the outermost axis and blends the two inner evaluations (lcms2's
    /// `Eval4InputsFloat`… recursion); at ≤ 3 dimensions dispatches to the mode's base case.
    fn eval_grid(&self, input: &[f64], axes: &[u8], table: &[f64], output: &mut [f64]) {
        match axes.len() {
            1 => self.eval_linear(input[0], axes[0], table, output),
            2 => self.eval_bilinear(input, axes, table, output),
            3 => match self.interpolation {
                ClutInterpolation::Multilinear => self.eval_trilinear(input, axes, table, output),
                ClutInterpolation::Tetrahedral => self.eval_tetrahedral(input, axes, table, output),
            },
            _ => {
                let out_n = output.len();
                // Stride of the outermost axis: one full inner sub-grid.
                let stride: usize =
                    axes[1..].iter().map(|&n| usize::from(n)).product::<usize>() * out_n;
                let (lower, upper, rest) = axis(input[0], axes[0]);
                let mut lo = [0.0_f64; MAX_CHANNELS as usize];
                let mut hi = [0.0_f64; MAX_CHANNELS as usize];
                self.eval_grid(
                    &input[1..],
                    &axes[1..],
                    &table[lower * stride..lower * stride + stride],
                    &mut lo[..out_n],
                );
                self.eval_grid(
                    &input[1..],
                    &axes[1..],
                    &table[upper * stride..upper * stride + stride],
                    &mut hi[..out_n],
                );
                for (out, (&y0, &y1)) in output.iter_mut().zip(lo.iter().zip(&hi)) {
                    // lcms2's blend: y0 + (y1 − y0) · rest.
                    *out = y0 + (y1 - y0) * rest;
                }
            }
        }
    }

    /// 1-D base case: linear interpolation between the two bracketing nodes
    /// (lcms2 `LinLerp1Dfloat`/`Eval1InputFloat`).
    fn eval_linear(&self, input: f64, nodes: u8, table: &[f64], output: &mut [f64]) {
        let out_n = output.len();
        let (lower, upper, r) = axis(input, nodes);
        for (ch, out) in output.iter_mut().enumerate() {
            let y0 = table[lower * out_n + ch];
            let y1 = table[upper * out_n + ch];
            *out = y0 + (y1 - y0) * r;
        }
    }

    /// 2-D base case: bilinear, `LERP` along X then Y (lcms2 `BilinearInterpFloat`).
    fn eval_bilinear(&self, input: &[f64], axes: &[u8], table: &[f64], output: &mut [f64]) {
        let out_n = output.len();
        let sx = usize::from(axes[1]) * out_n;
        let sy = out_n;
        let (x_lo, x_hi, fx) = axis(input[0], axes[0]);
        let (y_lo, y_hi, fy) = axis(input[1], axes[1]);
        let (x0, x1) = (x_lo * sx, x_hi * sx);
        let (y0, y1) = (y_lo * sy, y_hi * sy);
        for (ch, out) in output.iter_mut().enumerate() {
            let d00 = table[x0 + y0 + ch];
            let d01 = table[x0 + y1 + ch];
            let d10 = table[x1 + y0 + ch];
            let d11 = table[x1 + y1 + ch];
            let dx0 = lerp(fx, d00, d10);
            let dx1 = lerp(fx, d01, d11);
            *out = lerp(fy, dx0, dx1);
        }
    }

    /// 3-D multilinear base case: trilinear, `LERP` X then Y then Z
    /// (lcms2 `TrilinearInterpFloat`).
    fn eval_trilinear(&self, input: &[f64], axes: &[u8], table: &[f64], output: &mut [f64]) {
        let out_n = output.len();
        let sz = out_n;
        let sy = usize::from(axes[2]) * sz;
        let sx = usize::from(axes[1]) * sy;
        let (x_lo, x_hi, fx) = axis(input[0], axes[0]);
        let (y_lo, y_hi, fy) = axis(input[1], axes[1]);
        let (z_lo, z_hi, fz) = axis(input[2], axes[2]);
        let (x0, x1) = (x_lo * sx, x_hi * sx);
        let (y0, y1) = (y_lo * sy, y_hi * sy);
        let (z0, z1) = (z_lo * sz, z_hi * sz);
        for (ch, out) in output.iter_mut().enumerate() {
            let d000 = table[x0 + y0 + z0 + ch];
            let d001 = table[x0 + y0 + z1 + ch];
            let d010 = table[x0 + y1 + z0 + ch];
            let d011 = table[x0 + y1 + z1 + ch];
            let d100 = table[x1 + y0 + z0 + ch];
            let d101 = table[x1 + y0 + z1 + ch];
            let d110 = table[x1 + y1 + z0 + ch];
            let d111 = table[x1 + y1 + z1 + ch];
            let dx00 = lerp(fx, d000, d100);
            let dx01 = lerp(fx, d001, d101);
            let dx10 = lerp(fx, d010, d110);
            let dx11 = lerp(fx, d011, d111);
            let dxy0 = lerp(fy, dx00, dx10);
            let dxy1 = lerp(fy, dx01, dx11);
            *out = lerp(fz, dxy0, dxy1);
        }
    }

    /// 3-D tetrahedral base case: lcms2 `TetrahedralInterpFloat`'s six-branch cascade,
    /// `>=` comparisons in the same order (ties are order-dependent; see the transcription in
    /// `references/cmm/README.md`).
    ///
    /// lcms2 ends the cascade with an unreachable `c1 = c2 = c3 = 0` fallback for NaN
    /// fractions; `fclamp` maps NaN to `0.0` before the fractions exist, so for the finite
    /// fractions here the six orderings are exhaustive and the sixth is the `else` arm.
    fn eval_tetrahedral(&self, input: &[f64], axes: &[u8], table: &[f64], output: &mut [f64]) {
        let out_n = output.len();
        let sz = out_n;
        let sy = usize::from(axes[2]) * sz;
        let sx = usize::from(axes[1]) * sy;
        let (x_lo, x_hi, rx) = axis(input[0], axes[0]);
        let (y_lo, y_hi, ry) = axis(input[1], axes[1]);
        let (z_lo, z_hi, rz) = axis(input[2], axes[2]);
        let (x0, x1) = (x_lo * sx, x_hi * sx);
        let (y0, y1) = (y_lo * sy, y_hi * sy);
        let (z0, z1) = (z_lo * sz, z_hi * sz);
        for (ch, out) in output.iter_mut().enumerate() {
            let dens = |i: usize, j: usize, k: usize| table[i + j + k + ch];
            let c0 = dens(x0, y0, z0);
            let (c1, c2, c3) = if rx >= ry && ry >= rz {
                (
                    dens(x1, y0, z0) - c0,
                    dens(x1, y1, z0) - dens(x1, y0, z0),
                    dens(x1, y1, z1) - dens(x1, y1, z0),
                )
            } else if rx >= rz && rz >= ry {
                (
                    dens(x1, y0, z0) - c0,
                    dens(x1, y1, z1) - dens(x1, y0, z1),
                    dens(x1, y0, z1) - dens(x1, y0, z0),
                )
            } else if rz >= rx && rx >= ry {
                (
                    dens(x1, y0, z1) - dens(x0, y0, z1),
                    dens(x1, y1, z1) - dens(x1, y0, z1),
                    dens(x0, y0, z1) - c0,
                )
            } else if ry >= rx && rx >= rz {
                (
                    dens(x1, y1, z0) - dens(x0, y1, z0),
                    dens(x0, y1, z0) - c0,
                    dens(x1, y1, z1) - dens(x1, y1, z0),
                )
            } else if ry >= rz && rz >= rx {
                (
                    dens(x1, y1, z1) - dens(x0, y1, z1),
                    dens(x0, y1, z0) - c0,
                    dens(x0, y1, z1) - dens(x0, y1, z0),
                )
            } else {
                // rz >= ry && ry >= rx — the only ordering left once the five above fail.
                (
                    dens(x1, y1, z1) - dens(x0, y1, z1),
                    dens(x0, y1, z1) - dens(x0, y0, z1),
                    dens(x0, y0, z1) - c0,
                )
            };
            *out = c0 + c1 * rx + c2 * ry + c3 * rz;
        }
    }
}

#[cfg(test)]
mod tests {
    use gamut_icc::{Clut, ClutPrecision};

    use super::*;

    fn u16_clut(grid: &[u8], out: u8, samples: Vec<u16>) -> Clut {
        Clut {
            grid_points: grid.to_vec(),
            output_channels: out,
            precision: ClutPrecision::U16,
            samples,
        }
    }

    /// `u16` sample → the normalized `f64` node value a 16-bit table stores.
    fn norm(v: u16) -> f64 {
        f64::from(v) / 65535.0
    }

    /// A deterministic 64-bit LCG (Knuth's MMIX constants) for seeded random tables.
    struct Lcg(u64);

    impl Lcg {
        fn next_u32(&mut self) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) as u32
        }

        fn next_u16(&mut self) -> u16 {
            (self.next_u32() & 0xFFFF) as u16
        }

        /// A sample in `[0, 1]`-ish (occasionally slightly outside, exercising the clamp).
        fn next_unit(&mut self) -> f64 {
            f64::from(self.next_u32()) / f64::from(u32::MAX) * 1.1 - 0.05
        }
    }

    fn random_samples(seed: u64, n: usize) -> Vec<u16> {
        let mut lcg = Lcg(seed);
        (0..n).map(|_| lcg.next_u16()).collect()
    }

    /// Classic 2ᴺ-corner-weight multilinear interpolation, written independently of the
    /// production recursion (iterative corner enumeration, weight products) but sharing the
    /// documented input mapping (`fclamp`, floor cell, exact-1.0 and single-node edge rules).
    fn naive_multilinear(grid: &[u8], out: usize, samples: &[u16], input: &[f64]) -> Vec<f64> {
        let dims = grid.len();
        let mut lo = vec![0_usize; dims];
        let mut hi = vec![0_usize; dims];
        let mut fr = vec![0.0_f64; dims];
        for d in 0..dims {
            let v = input[d];
            let c = if v.is_nan() || v < 1.0e-9 {
                0.0
            } else if v > 1.0 {
                1.0
            } else {
                v
            };
            let domain = usize::from(grid[d]) - 1;
            let px = c * domain as f64;
            lo[d] = px.floor() as usize;
            fr[d] = px - px.floor();
            hi[d] = if domain == 0 || c >= 1.0 {
                lo[d]
            } else {
                lo[d] + 1
            };
        }
        let strides: Vec<usize> = (0..dims)
            .map(|d| {
                out * grid[d + 1..]
                    .iter()
                    .map(|&n| usize::from(n))
                    .product::<usize>()
            })
            .collect();
        let mut result = vec![0.0_f64; out];
        for corner in 0..(1_usize << dims) {
            let mut weight = 1.0;
            let mut base = 0_usize;
            for d in 0..dims {
                if corner & (1 << d) != 0 {
                    weight *= fr[d];
                    base += hi[d] * strides[d];
                } else {
                    weight *= 1.0 - fr[d];
                    base += lo[d] * strides[d];
                }
            }
            for (ch, acc) in result.iter_mut().enumerate() {
                *acc += weight * norm(samples[base + ch]);
            }
        }
        result
    }

    /// The 2×2×2 corner set of the tetrahedral probes, in grid order (last axis fastest):
    /// `d000, d001, d010, d011, d100, d101, d110, d111`. Chosen (empirically) so that at every
    /// tie probe below the two branch formulas adjacent under an `>=`→`>` swap give
    /// *bitwise-different* `f64` results — exact-equality assertions then pin lcms2's tie
    /// resolution, not just continuity.
    const TETRA_CORNERS: [u16; 8] = [39495, 16622, 30837, 48671, 9212, 18347, 44806, 19172];

    fn tetra_table() -> ClutTable {
        ClutTable::new(&u16_clut(&[2, 2, 2], 1, TETRA_CORNERS.to_vec())).unwrap()
    }

    /// Evaluates a 1-output table at one point.
    fn eval1(table: &ClutTable, input: &[f64]) -> f64 {
        let mut out = [0.0];
        table.eval(input, &mut out);
        out[0]
    }

    // ---- addressing --------------------------------------------------------------------------

    #[test]
    fn node_lookups_pin_last_axis_fastest_addressing_and_output_interleaving() {
        // 2×3×2 grid, 2 outputs: sample value encodes (node index, channel) so any addressing
        // slip lands on a different, distinct value. Node index (x,y,z) = ((x·3)+y)·2+z.
        let samples: Vec<u16> = (0..12 * 2).map(|i| 1000 + 137 * i as u16).collect();
        let clut = u16_clut(&[2, 3, 2], 2, samples.clone());
        for mode in [
            ClutInterpolation::Multilinear,
            ClutInterpolation::Tetrahedral,
        ] {
            let table = ClutTable::with_interpolation(&clut, mode).unwrap();
            for (x, xi) in [(0.0, 0_usize), (1.0, 1)] {
                for (y, yi) in [(0.0, 0_usize), (0.5, 1), (1.0, 2)] {
                    for (z, zi) in [(0.0, 0_usize), (1.0, 1)] {
                        let node = (xi * 3 + yi) * 2 + zi;
                        let mut out = [9.0; 2];
                        table.eval(&[x, y, z], &mut out);
                        // On-node fractions are all zero, so the stored sample comes back
                        // exactly (both interpolants reduce to the node value).
                        assert_eq!(out[0], norm(samples[node * 2]), "{mode:?} node {node}");
                        assert_eq!(out[1], norm(samples[node * 2 + 1]), "{mode:?} node {node}");
                    }
                }
            }
        }
    }

    #[test]
    fn node_lookups_1d_and_4d() {
        // 1-D, 3 nodes, 2 outputs.
        let samples: Vec<u16> = vec![100, 200, 30000, 31000, 64000, 65000];
        let table = ClutTable::new(&u16_clut(&[3], 2, samples.clone())).unwrap();
        for (v, node) in [(0.0, 0_usize), (0.5, 1), (1.0, 2)] {
            let mut out = [9.0; 2];
            table.eval(&[v], &mut out);
            assert_eq!(out, [norm(samples[node * 2]), norm(samples[node * 2 + 1])]);
        }
        // 4-D, 2 nodes per axis, 1 output: every corner comes back exactly.
        let samples = random_samples(11, 16);
        let table = ClutTable::new(&u16_clut(&[2, 2, 2, 2], 1, samples.clone())).unwrap();
        for (corner, &sample) in samples.iter().enumerate() {
            let input: Vec<f64> = (0..4)
                .map(|d| if corner & (8 >> d) != 0 { 1.0 } else { 0.0 })
                .collect();
            assert_eq!(eval1(&table, &input), norm(sample), "corner {corner}");
        }
    }

    #[test]
    fn u8_precision_normalizes_by_255_not_65535() {
        let clut = Clut {
            grid_points: vec![2],
            output_channels: 1,
            precision: ClutPrecision::U8,
            samples: vec![51, 204], // 8-bit data widened to u16 by the parser, still 0..=255
        };
        let table = ClutTable::new(&clut).unwrap();
        assert_eq!(eval1(&table, &[0.0]), 51.0 / 255.0);
        assert_eq!(eval1(&table, &[1.0]), 204.0 / 255.0);
        assert_eq!(eval1(&table, &[0.5]), 0.2 + (0.8 - 0.2) * 0.5);
    }

    // ---- multilinear -------------------------------------------------------------------------

    #[test]
    fn linear_1d_matches_hand_interpolation() {
        let table = ClutTable::new(&u16_clut(&[3], 1, vec![4000, 30000, 52000])).unwrap();
        // 0.25 → px = 0.5: halfway between nodes 0 and 1 (identical expression shape, so ==).
        assert_eq!(
            eval1(&table, &[0.25]),
            norm(4000) + (norm(30000) - norm(4000)) * 0.5
        );
        // 0.75 → px = 1.5: halfway between nodes 1 and 2.
        assert_eq!(
            eval1(&table, &[0.75]),
            norm(30000) + (norm(52000) - norm(30000)) * 0.5
        );
    }

    #[test]
    fn bilinear_2d_matches_hand_lerp_x_then_y() {
        // 2×3 grid, 2 outputs; probe the (y ∈ [0, 0.5]) cell at fx = 0.5, fy = 0.5.
        let samples = random_samples(23, 12);
        let table = ClutTable::new(&u16_clut(&[2, 3], 2, samples.clone())).unwrap();
        let mut out = [9.0; 2];
        table.eval(&[0.5, 0.25], &mut out);
        for ch in 0..2 {
            let d = |x: usize, y: usize| norm(samples[(x * 3 + y) * 2 + ch]);
            let dx0 = d(0, 0) + (d(1, 0) - d(0, 0)) * 0.5;
            let dx1 = d(0, 1) + (d(1, 1) - d(0, 1)) * 0.5;
            assert_eq!(out[ch], dx0 + (dx1 - dx0) * 0.5, "channel {ch}");
        }

        // Order-pinning probe: corners chosen (empirically) so lcms2's X-then-Y lerp order
        // differs *bitwise* from lerping Y inside each X-slice and blending along X — the
        // exact-equality assertion therefore fails if the 2-D base case is ever routed
        // through the generic slice-and-blend recursion instead of `BilinearInterpFloat`'s
        // order.
        let (d00, d01, d10, d11) = (47062_u16, 17241, 28876, 43750);
        let table = ClutTable::new(&u16_clut(&[2, 2], 1, vec![d00, d01, d10, d11])).unwrap();
        let dx0 = norm(d00) + (norm(d10) - norm(d00)) * 0.5;
        let dx1 = norm(d01) + (norm(d11) - norm(d01)) * 0.5;
        assert_eq!(eval1(&table, &[0.5, 0.25]), dx0 + (dx1 - dx0) * 0.25);
    }

    #[test]
    fn multilinear_matches_naive_corner_weights_up_to_4d() {
        // Every axis has ≥ 3 nodes so random interior points give *nonzero* lower node
        // offsets on every axis — index arithmetic (stride sums) is exercised away from the
        // origin cell, where wrong signs would go unnoticed.
        let grids: [&[u8]; 4] = [&[5], &[3, 4], &[4, 3, 3], &[3, 3, 4, 3]];
        for (g, grid) in grids.iter().enumerate() {
            let out = 2_usize;
            let dims = grid.len();
            let nodes: usize = grid.iter().map(|&n| usize::from(n)).product();
            let samples = random_samples(100 + g as u64, nodes * out);
            let clut = u16_clut(grid, out as u8, samples.clone());
            let table =
                ClutTable::with_interpolation(&clut, ClutInterpolation::Multilinear).unwrap();
            let mut lcg = Lcg(31 + g as u64);
            let mut points: Vec<Vec<f64>> = (0..40)
                .map(|_| (0..dims).map(|_| lcg.next_unit()).collect())
                .collect();
            // Per-axis exact-1.0 probes with the other axes interior: the top grid plane has
            // maximal node offsets on that axis, which distinguishes +/− in every stride sum.
            for d in 0..dims {
                let mut p: Vec<f64> = (0..dims).map(|k| 0.37 + 0.11 * (k % 3) as f64).collect();
                p[d] = 1.0;
                points.push(p);
            }
            for (p, input) in points.iter().enumerate() {
                let want = naive_multilinear(grid, out, &samples, input);
                let mut got = [9.0; 2];
                table.eval(input, &mut got);
                for ch in 0..out {
                    assert!(
                        (got[ch] - want[ch]).abs() < 1e-12,
                        "{dims}-D point {p} ch {ch}: {} vs {}",
                        got[ch],
                        want[ch]
                    );
                }
            }
        }
    }

    // ---- tetrahedral -------------------------------------------------------------------------

    /// One interior probe per branch of the cascade, asserted bitwise against the branch's
    /// hand-transcribed `c0 + c1·rx + c2·ry + c3·rz` formula (`cmsintrp.c`
    /// `TetrahedralInterpFloat`). Any wrong branch or wrong corner difference lands on a
    /// different value of order the corner spread, not an ulp.
    #[test]
    fn tetrahedral_interior_probes_match_hand_branch_formulas() {
        let t = tetra_table();
        let [d000, d001, d010, d011, d100, d101, d110, d111] = TETRA_CORNERS.map(norm);
        let c0 = d000;
        // Branch 1: rx ≥ ry ≥ rz.
        assert_eq!(
            eval1(&t, &[0.75, 0.5, 0.25]),
            c0 + (d100 - c0) * 0.75 + (d110 - d100) * 0.5 + (d111 - d110) * 0.25
        );
        // Branch 2: rx ≥ rz ≥ ry.
        assert_eq!(
            eval1(&t, &[0.75, 0.25, 0.5]),
            c0 + (d100 - c0) * 0.75 + (d111 - d101) * 0.25 + (d101 - d100) * 0.5
        );
        // Branch 3: rz ≥ rx ≥ ry.
        assert_eq!(
            eval1(&t, &[0.5, 0.25, 0.75]),
            c0 + (d101 - d001) * 0.5 + (d111 - d101) * 0.25 + (d001 - c0) * 0.75
        );
        // Branch 4: ry ≥ rx ≥ rz.
        assert_eq!(
            eval1(&t, &[0.5, 0.75, 0.25]),
            c0 + (d110 - d010) * 0.5 + (d010 - c0) * 0.75 + (d111 - d110) * 0.25
        );
        // Branch 5: ry ≥ rz ≥ rx.
        assert_eq!(
            eval1(&t, &[0.25, 0.75, 0.5]),
            c0 + (d111 - d011) * 0.25 + (d010 - c0) * 0.75 + (d011 - d010) * 0.5
        );
        // Branch 6: rz ≥ ry ≥ rx.
        assert_eq!(
            eval1(&t, &[0.25, 0.5, 0.75]),
            c0 + (d111 - d011) * 0.25 + (d011 - d001) * 0.5 + (d001 - c0) * 0.75
        );
    }

    /// Exact ties resolve to the *earlier* branch, as in lcms2's `>=` cascade. At a tie the
    /// two adjacent branch formulas agree mathematically but (for these corners, verified)
    /// not bitwise — so the exact-equality assertions distinguish `>=` from `>` at every
    /// reachable tie.
    #[test]
    fn tetrahedral_tie_probes_resolve_in_lcms2_branch_order() {
        let t = tetra_table();
        let [d000, d001, d010, d011, d100, d101, d110, d111] = TETRA_CORNERS.map(norm);
        let c0 = d000;
        let (tt, s, r) = (0.6, 0.3, 0.9);
        // rx == ry > rz → branch 1 (not 4).
        assert_eq!(
            eval1(&t, &[tt, tt, s]),
            c0 + (d100 - c0) * tt + (d110 - d100) * tt + (d111 - d110) * s
        );
        // rx > ry == rz → branch 1 (not 2).
        assert_eq!(
            eval1(&t, &[r, tt, tt]),
            c0 + (d100 - c0) * r + (d110 - d100) * tt + (d111 - d110) * tt
        );
        // rx == rz > ry → branch 2 (not 3).
        assert_eq!(
            eval1(&t, &[tt, s, tt]),
            c0 + (d100 - c0) * tt + (d111 - d101) * s + (d101 - d100) * tt
        );
        // rz > rx == ry → branch 3 (not 6).
        assert_eq!(
            eval1(&t, &[s, s, r]),
            c0 + (d101 - d001) * s + (d111 - d101) * s + (d001 - c0) * r
        );
        // ry > rx == rz → branch 4 (not 5).
        assert_eq!(
            eval1(&t, &[s, r, s]),
            c0 + (d110 - d010) * s + (d010 - c0) * r + (d111 - d110) * s
        );
        // ry == rz > rx → branch 5 (not 6).
        assert_eq!(
            eval1(&t, &[s, tt, tt]),
            c0 + (d111 - d011) * s + (d010 - c0) * tt + (d011 - d010) * tt
        );
        // rx == ry == rz → branch 1: linear along the cell's main diagonal.
        assert_eq!(
            eval1(&t, &[tt, tt, tt]),
            c0 + (d100 - c0) * tt + (d110 - d100) * tt + (d111 - d110) * tt
        );
    }

    #[test]
    fn tetrahedral_equals_trilinear_on_nodes_and_edges() {
        let clut = u16_clut(&[2, 2, 2], 1, TETRA_CORNERS.to_vec());
        let tetra = ClutTable::new(&clut).unwrap();
        let tri = ClutTable::with_interpolation(&clut, ClutInterpolation::Multilinear).unwrap();
        // All 8 nodes and the midpoint of all 12 cell edges: the two interpolants coincide
        // exactly (on an edge both reduce to the same single lerp).
        for x in [0.0, 0.5, 1.0] {
            for y in [0.0, 0.5, 1.0] {
                for z in [0.0, 0.5, 1.0] {
                    let interior = (x == 0.5) as u8 + (y == 0.5) as u8 + (z == 0.5) as u8;
                    if interior > 1 {
                        continue; // faces and the centre differ; asserted separately below
                    }
                    assert_eq!(
                        eval1(&tetra, &[x, y, z]),
                        eval1(&tri, &[x, y, z]),
                        "at ({x}, {y}, {z})"
                    );
                }
            }
        }
    }

    #[test]
    fn tetrahedral_differs_from_trilinear_off_edge_within_bound() {
        // A seeded random 5×5×5 grid: measure max |tetrahedral − trilinear| over a sweep.
        // Both interpolants are convex combinations of the same cell's 8 corners, so the
        // difference is bounded by the in-cell corner spread (< 1 for normalized samples);
        // measured max here is 0.3199. Asserted: measurably nonzero somewhere, and under 0.4.
        let samples = random_samples(7, 125);
        let clut = u16_clut(&[5, 5, 5], 1, samples);
        let tetra = ClutTable::new(&clut).unwrap();
        let tri = ClutTable::with_interpolation(&clut, ClutInterpolation::Multilinear).unwrap();
        let mut lcg = Lcg(99);
        let mut max_diff = 0.0_f64;
        for _ in 0..500 {
            let input = [lcg.next_unit(), lcg.next_unit(), lcg.next_unit()];
            let diff = (eval1(&tetra, &input) - eval1(&tri, &input)).abs();
            max_diff = max_diff.max(diff);
        }
        assert!(
            max_diff > 1e-3,
            "tetrahedral should differ from trilinear off-edge, max {max_diff:e}"
        );
        assert!(max_diff < 0.4, "documented bound exceeded: {max_diff}");
    }

    // ---- input mapping edges ----------------------------------------------------------------

    #[test]
    fn fclamp_maps_nan_negatives_and_sub_epsilon_to_zero() {
        // 1-D identity ramp: output equals the clamped input exactly.
        let table = ClutTable::new(&u16_clut(&[2], 1, vec![0, 65535])).unwrap();
        assert_eq!(eval1(&table, &[f64::NAN]), 0.0);
        assert_eq!(eval1(&table, &[-3.0]), 0.0);
        assert_eq!(eval1(&table, &[-0.0]), 0.0);
        assert_eq!(eval1(&table, &[5e-10]), 0.0); // below lcms2's 1e-9 threshold
        assert_eq!(eval1(&table, &[1e-9]), 1e-9); // at the threshold: kept
        assert_eq!(eval1(&table, &[0.75]), 0.75);
        assert_eq!(eval1(&table, &[1.0]), 1.0);
        assert_eq!(eval1(&table, &[2.5]), 1.0); // above 1.0: clamped
    }

    #[test]
    fn exact_one_lands_on_the_last_node_in_every_mode() {
        // 3 nodes per axis so 1.0 sits on the grid boundary where the upper-index rule
        // matters; the top-corner sample must come back exactly, not read out of bounds.
        let samples = random_samples(55, 27);
        let clut = u16_clut(&[3, 3, 3], 1, samples.clone());
        for mode in [
            ClutInterpolation::Multilinear,
            ClutInterpolation::Tetrahedral,
        ] {
            let table = ClutTable::with_interpolation(&clut, mode).unwrap();
            assert_eq!(
                eval1(&table, &[1.0, 1.0, 1.0]),
                norm(samples[26]),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn single_node_axes_interpolate_as_constant() {
        // 1-D single node: every input yields the node (lcms2's guarded 1-D case).
        let table = ClutTable::new(&u16_clut(&[1], 1, vec![40000])).unwrap();
        for v in [0.0, 0.3, 1.0, f64::NAN] {
            assert_eq!(eval1(&table, &[v]), norm(40000));
        }
        // 3-D with a single-node middle axis, both modes: the documented divergence — lcms2
        // has no Domain == 0 guard here (out-of-bounds territory); we pin constancy along the
        // degenerate axis.
        let samples = random_samples(77, 4);
        let clut = u16_clut(&[2, 1, 2], 1, samples);
        for mode in [
            ClutInterpolation::Multilinear,
            ClutInterpolation::Tetrahedral,
        ] {
            let table = ClutTable::with_interpolation(&clut, mode).unwrap();
            let base = eval1(&table, &[0.4, 0.0, 0.9]);
            for y in [0.3, 0.7, 1.0] {
                assert_eq!(eval1(&table, &[0.4, y, 0.9]), base, "{mode:?} y = {y}");
            }
        }
    }

    // ---- high dimensions --------------------------------------------------------------------

    #[test]
    fn fifteen_dimensions_accepted_and_evaluated() {
        let samples = random_samples(13, 1 << 15);
        let clut = u16_clut(&[2; 15], 1, samples.clone());
        let table = ClutTable::new(&clut).unwrap();
        // lcms2's default for ≥ 3 inputs.
        assert_eq!(table.interpolation(), ClutInterpolation::Tetrahedral);
        assert_eq!(table.input_channels(), 15);
        assert_eq!(table.output_channels(), 1);
        // Corner lookups address the first and last node exactly in both modes.
        let multi = ClutTable::with_interpolation(&clut, ClutInterpolation::Multilinear).unwrap();
        for t in [&table, &multi] {
            assert_eq!(eval1(t, &[0.0; 15]), norm(samples[0]));
            assert_eq!(eval1(t, &[1.0; 15]), norm(samples[(1 << 15) - 1]));
        }
        // Multilinear at the centre of a 2-node-per-axis grid weights every corner by 2⁻¹⁵ —
        // the grid mean — pinning the full recursion depth in one closed form.
        let mean = samples.iter().map(|&s| norm(s)).sum::<f64>() / f64::from(1 << 15);
        assert!(
            (eval1(&multi, &[0.5; 15]) - mean).abs() < 1e-9,
            "centre {} vs mean {mean}",
            eval1(&multi, &[0.5; 15])
        );
    }

    // ---- construction validation ------------------------------------------------------------

    #[test]
    fn default_mode_is_tetrahedral_from_three_inputs() {
        let one = ClutTable::new(&u16_clut(&[2], 1, vec![0, 1])).unwrap();
        assert_eq!(one.interpolation(), ClutInterpolation::Multilinear);
        let two = ClutTable::new(&u16_clut(&[2, 2], 1, vec![0; 4])).unwrap();
        assert_eq!(two.interpolation(), ClutInterpolation::Multilinear);
        let three = ClutTable::new(&u16_clut(&[2, 2, 2], 1, vec![0; 8])).unwrap();
        assert_eq!(three.interpolation(), ClutInterpolation::Tetrahedral);
        let four = ClutTable::new(&u16_clut(&[2, 2, 2, 2], 1, vec![0; 16])).unwrap();
        assert_eq!(four.interpolation(), ClutInterpolation::Tetrahedral);
    }

    #[test]
    fn channel_accessors_report_grid_shape() {
        let table = ClutTable::new(&u16_clut(&[2, 3, 2], 4, vec![0; 48])).unwrap();
        assert_eq!(table.input_channels(), 3);
        assert_eq!(table.output_channels(), 4);
    }

    #[test]
    fn geometry_validation_rejects_inconsistent_tables() {
        let err = ClutTable::new(&u16_clut(&[], 1, vec![])).unwrap_err();
        assert!(matches!(err, CmmError::ClutGeometry("no input dimensions")));

        let err = ClutTable::new(&u16_clut(&[2; 16], 1, vec![0; 1 << 16])).unwrap_err();
        assert!(matches!(err, CmmError::TooManyChannels(16)));

        let err = ClutTable::new(&u16_clut(&[2, 0, 2], 1, vec![])).unwrap_err();
        assert!(matches!(err, CmmError::ClutGeometry("zero grid axis")));

        let err = ClutTable::new(&u16_clut(&[2, 2], 1, vec![0; 5])).unwrap_err();
        assert!(matches!(
            err,
            CmmError::ClutGeometry("sample count mismatch")
        ));

        // Node-count product overflow can never match a real sample count.
        let err = ClutTable::new(&u16_clut(&[255; 15], 1, vec![0; 8])).unwrap_err();
        assert!(matches!(
            err,
            CmmError::ClutGeometry("sample count mismatch")
        ));

        let err = ClutTable::new(&u16_clut(&[2, 2], 0, vec![])).unwrap_err();
        assert!(matches!(err, CmmError::TooManyChannels(0)));
        let err = ClutTable::new(&u16_clut(&[2, 2], 17, vec![0; 68])).unwrap_err();
        assert!(matches!(err, CmmError::TooManyChannels(17)));
        // 16 outputs is the accepted boundary.
        let table = ClutTable::new(&u16_clut(&[2, 2, 2], 16, vec![0; 128])).unwrap();
        assert_eq!(table.output_channels(), 16);
    }

    #[test]
    fn tetrahedral_below_three_inputs_is_rejected() {
        for grid in [&[5_u8] as &[u8], &[3, 3]] {
            let nodes: usize = grid.iter().map(|&n| usize::from(n)).product();
            let clut = u16_clut(grid, 1, vec![0; nodes]);
            let err =
                ClutTable::with_interpolation(&clut, ClutInterpolation::Tetrahedral).unwrap_err();
            assert!(
                matches!(
                    err,
                    CmmError::ClutGeometry(
                        "tetrahedral interpolation requires at least 3 input channels"
                    )
                ),
                "{}-D: {err:?}",
                grid.len()
            );
        }
    }

    #[test]
    fn clut_geometry_error_message_names_the_cause() {
        assert_eq!(
            CmmError::ClutGeometry("zero grid axis").to_string(),
            "cmm: CLUT geometry inconsistent (zero grid axis)"
        );
    }
}
