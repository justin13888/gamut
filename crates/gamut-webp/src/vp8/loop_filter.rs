//! VP8 in-loop deblocking filter (RFC 6386 §15).
//!
//! After reconstruction, VP8 smooths macroblock and subblock edges. Crucially the filter runs as a
//! **final whole-frame pass** — intra prediction (§12) reads the *unfiltered* reconstruction, so the
//! encoder reconstructs the frame unfiltered (predicting from it) and only then deblocks the output.
//!
//! This module implements both the **simple filter** (§15.2 — luma edges only) and the **normal
//! filter** (§15.3 — luma and chroma, with a high-edge-variance test that selects between a six-tap
//! macroblock-edge and a four-tap subblock-edge adjustment). Control parameters (the interior, edge,
//! and HEV limits) derive from the frame's `loop_filter_level` and `sharpness_level` per §15.4.
//! Tracked in `../STATUS.md` section M.

/// Clamps an intermediate to the signed 8-bit range (RFC 6386 §15.2 `c`).
fn c(v: i32) -> i32 {
    v.clamp(-128, 127)
}

/// Converts an unsigned pixel to the signed-centered domain (`u2s`).
fn u2s(v: u8) -> i32 {
    i32::from(v) - 128
}

/// Clamps and converts a signed value back to an unsigned pixel (`s2u`).
fn s2u(v: i32) -> u8 {
    (c(v) + 128) as u8
}

/// The common 2-or-4-tap edge adjustment (RFC 6386 §15.2 `common_adjust`) on the four pixels at
/// `p1`/`p0`/`q0`/`q1` of `plane`. With `use_outer_taps` the outer pixels participate (the simple
/// filter always passes `true`). Returns the adjustment applied to `q0`.
fn common_adjust(
    use_outer_taps: bool,
    plane: &mut [u8],
    p1: usize,
    p0: usize,
    q0: usize,
    q1: usize,
) -> i32 {
    let (sp1, sp0, sq0, sq1) = (
        u2s(plane[p1]),
        u2s(plane[p0]),
        u2s(plane[q0]),
        u2s(plane[q1]),
    );
    let a = c((if use_outer_taps { c(sp1 - sq1) } else { 0 }) + 3 * (sq0 - sp0));
    let b = c(a + 3) >> 3;
    let a = c(a + 4) >> 3;
    plane[q0] = s2u(sq0 - a);
    plane[p0] = s2u(sp0 + b);
    a
}

/// Filters one segment with the simple filter (RFC 6386 §15.2 `simple_segment`): adjust the four
/// pixels only if the edge difference is within `edge_limit`.
fn simple_segment(edge_limit: i32, plane: &mut [u8], p1: usize, p0: usize, q0: usize, q1: usize) {
    let diff = (i32::from(plane[p0]) - i32::from(plane[q0])).abs() * 2
        + (i32::from(plane[p1]) - i32::from(plane[q1])).abs() / 2;
    if diff <= edge_limit {
        common_adjust(true, plane, p1, p0, q0, q1);
    }
}

/// The interior difference limit (RFC 6386 §15.4): derived from level and sharpness.
fn interior_limit(level: u8, sharpness: u8) -> i32 {
    let mut limit = i32::from(level);
    if sharpness > 0 {
        limit >>= if sharpness > 4 { 2 } else { 1 };
        let cap = 9 - i32::from(sharpness);
        if limit > cap {
            limit = cap;
        }
    }
    limit.max(1)
}

/// Filters the 16-segment vertical edge at column `ex` (rows `py..py+16`); each segment straddles the
/// edge as `(ex-2, ex-1 | ex, ex+1)`.
fn filter_vedge(plane: &mut [u8], stride: usize, ex: usize, py: usize, edge_limit: i32) {
    for r in 0..16 {
        let base = (py + r) * stride + ex;
        simple_segment(edge_limit, plane, base - 2, base - 1, base, base + 1);
    }
}

/// Filters the 16-segment horizontal edge at row `ey` (columns `px..px+16`); each segment straddles
/// the edge vertically as `(ey-2, ey-1 | ey, ey+1)`.
fn filter_hedge(plane: &mut [u8], stride: usize, px: usize, ey: usize, edge_limit: i32) {
    for col in 0..16 {
        let base = ey * stride + px + col;
        simple_segment(
            edge_limit,
            plane,
            base - 2 * stride,
            base - stride,
            base,
            base + stride,
        );
    }
}

/// Applies the simple loop filter to a macroblock-aligned luma `plane` (RFC 6386 §15.1/§15.2). Each
/// macroblock filters, in order, its left and top inter-macroblock edges (always) and its interior
/// vertical/horizontal subblock edges (only where `filter_interior[mb]` is set — i.e. the macroblock
/// is `B_PRED` or carries coefficients), at its own `mb_level[mb]` (a level of 0 skips it; per-segment
/// filter levels make this vary by macroblock). Chroma is left unfiltered.
pub fn simple_filter_luma(
    plane: &mut [u8],
    stride: usize,
    mb_cols: usize,
    mb_rows: usize,
    mb_level: &[u8],
    sharpness: u8,
    filter_interior: &[bool],
) {
    for mb_y in 0..mb_rows {
        for mb_x in 0..mb_cols {
            let level = mb_level[mb_y * mb_cols + mb_x];
            if level == 0 {
                continue;
            }
            let interior = interior_limit(level, sharpness);
            let mbedge_limit = (i32::from(level) + 2) * 2 + interior;
            let sub_bedge_limit = i32::from(level) * 2 + interior;
            let (px, py) = (mb_x * 16, mb_y * 16);
            let do_interior = filter_interior[mb_y * mb_cols + mb_x];
            if mb_x > 0 {
                filter_vedge(plane, stride, px, py, mbedge_limit);
            }
            if do_interior {
                for dx in [4, 8, 12] {
                    filter_vedge(plane, stride, px + dx, py, sub_bedge_limit);
                }
            }
            if mb_y > 0 {
                filter_hedge(plane, stride, px, py, mbedge_limit);
            }
            if do_interior {
                for dy in [4, 8, 12] {
                    filter_hedge(plane, stride, px, py + dy, sub_bedge_limit);
                }
            }
        }
    }
}

/// Whether to filter the segment at all (RFC 6386 §15.3 `filter_yes`): the edge difference must be
/// within `edge` and every adjacent interior difference within `interior`. Indices name the eight
/// segment pixels `p3 p2 p1 p0 | q0 q1 q2 q3`.
#[allow(clippy::too_many_arguments)]
fn filter_yes(
    interior: i32,
    edge: i32,
    plane: &[u8],
    p3: usize,
    p2: usize,
    p1: usize,
    p0: usize,
    q0: usize,
    q1: usize,
    q2: usize,
    q3: usize,
) -> bool {
    let d = |a: usize, b: usize| (i32::from(plane[a]) - i32::from(plane[b])).abs();
    (d(p0, q0) * 2 + d(p1, q1) / 2) <= edge
        && d(p3, p2) <= interior
        && d(p2, p1) <= interior
        && d(p1, p0) <= interior
        && d(q3, q2) <= interior
        && d(q2, q1) <= interior
        && d(q1, q0) <= interior
}

/// The high-edge-variance test (RFC 6386 §15.3 `hev`): true if either inner difference exceeds the
/// threshold, which steers the normal filter toward the simpler outer-tap adjustment.
fn hev(threshold: i32, p1: u8, p0: u8, q0: u8, q1: u8) -> bool {
    (i32::from(p1) - i32::from(p0)).abs() > threshold
        || (i32::from(q1) - i32::from(q0)).abs() > threshold
}

/// The eight segment indices straddling an edge at `idx` with `step` between consecutive pixels
/// (`1` for a vertical edge, `stride` for a horizontal one): `(p3, p2, p1, p0, q0, q1, q2, q3)`.
fn segment_indices(
    idx: usize,
    step: usize,
) -> (usize, usize, usize, usize, usize, usize, usize, usize) {
    (
        idx - 4 * step,
        idx - 3 * step,
        idx - 2 * step,
        idx - step,
        idx,
        idx + step,
        idx + 2 * step,
        idx + 3 * step,
    )
}

/// Normal inter-subblock filter for one segment (RFC 6386 §15.3 `subblock_filter`): the simple
/// adjustment, plus a half-strength adjustment of the next pixels in when edge variance is low.
fn normal_subblock_segment(
    plane: &mut [u8],
    idx: usize,
    step: usize,
    edge: i32,
    interior: i32,
    hev_t: i32,
) {
    let (p3, p2, p1, p0, q0, q1, q2, q3) = segment_indices(idx, step);
    if !filter_yes(interior, edge, plane, p3, p2, p1, p0, q0, q1, q2, q3) {
        return;
    }
    let hv = hev(hev_t, plane[p1], plane[p0], plane[q0], plane[q1]);
    let a = (common_adjust(hv, plane, p1, p0, q0, q1) + 1) >> 1;
    if !hv {
        plane[q1] = s2u(u2s(plane[q1]) - a);
        plane[p1] = s2u(u2s(plane[p1]) + a);
    }
}

/// Normal inter-macroblock filter for one segment (RFC 6386 §15.3 `MBfilter`): a six-tap adjustment
/// with magnitude decaying away from the edge when variance is low, else the simple adjustment.
fn normal_mb_segment(
    plane: &mut [u8],
    idx: usize,
    step: usize,
    edge: i32,
    interior: i32,
    hev_t: i32,
) {
    let (p3, p2, p1, p0, q0, q1, q2, q3) = segment_indices(idx, step);
    if !filter_yes(interior, edge, plane, p3, p2, p1, p0, q0, q1, q2, q3) {
        return;
    }
    if hev(hev_t, plane[p1], plane[p0], plane[q0], plane[q1]) {
        common_adjust(true, plane, p1, p0, q0, q1);
        return;
    }
    let (sp2, sp1, sp0) = (u2s(plane[p2]), u2s(plane[p1]), u2s(plane[p0]));
    let (sq0, sq1, sq2) = (u2s(plane[q0]), u2s(plane[q1]), u2s(plane[q2]));
    let w = c(c(sp1 - sq1) + 3 * (sq0 - sp0));
    let a = c((27 * w + 63) >> 7);
    plane[q0] = s2u(sq0 - a);
    plane[p0] = s2u(sp0 + a);
    let a = c((18 * w + 63) >> 7);
    plane[q1] = s2u(sq1 - a);
    plane[p1] = s2u(sp1 + a);
    let a = c((9 * w + 63) >> 7);
    plane[q2] = s2u(sq2 - a);
    plane[p2] = s2u(sp2 + a);
}

/// The key-frame high-edge-variance threshold from the filter level (RFC 6386 §15.4).
fn keyframe_hev_threshold(level: u8) -> i32 {
    if level >= 40 {
        2
    } else if level >= 15 {
        1
    } else {
        0
    }
}

/// Filters one macroblock of a plane with the normal filter: the left/top inter-macroblock edges
/// (six-tap) and — where `interior_edges` — the interior subblock edges (four-tap). `size` is 16 for
/// luma or 8 for chroma; `sub` the interior edge offsets (`[4,8,12]` luma, `[4]` chroma).
#[allow(clippy::too_many_arguments)]
fn normal_filter_plane(
    plane: &mut [u8],
    stride: usize,
    px: usize,
    py: usize,
    size: usize,
    sub: &[usize],
    left: bool,
    top: bool,
    interior_edges: bool,
    mbedge: i32,
    sub_bedge: i32,
    interior: i32,
    hev_t: i32,
) {
    if left {
        for r in 0..size {
            normal_mb_segment(plane, (py + r) * stride + px, 1, mbedge, interior, hev_t);
        }
    }
    if interior_edges {
        for &dx in sub {
            for r in 0..size {
                normal_subblock_segment(
                    plane,
                    (py + r) * stride + px + dx,
                    1,
                    sub_bedge,
                    interior,
                    hev_t,
                );
            }
        }
    }
    if top {
        for col in 0..size {
            normal_mb_segment(
                plane,
                py * stride + px + col,
                stride,
                mbedge,
                interior,
                hev_t,
            );
        }
    }
    if interior_edges {
        for &dy in sub {
            for col in 0..size {
                normal_subblock_segment(
                    plane,
                    (py + dy) * stride + px + col,
                    stride,
                    sub_bedge,
                    interior,
                    hev_t,
                );
            }
        }
    }
}

/// Applies the normal loop filter to the macroblock-aligned Y, U, and V planes (RFC 6386 §15.3),
/// macroblock by macroblock in raster order. Unlike the simple filter this deblocks chroma too. A
/// zero `level` is a no-op.
#[allow(clippy::too_many_arguments)]
pub fn normal_filter(
    y: &mut [u8],
    u: &mut [u8],
    v: &mut [u8],
    y_stride: usize,
    c_stride: usize,
    mb_cols: usize,
    mb_rows: usize,
    mb_level: &[u8],
    sharpness: u8,
    filter_interior: &[bool],
) {
    for mb_y in 0..mb_rows {
        for mb_x in 0..mb_cols {
            let level = mb_level[mb_y * mb_cols + mb_x];
            if level == 0 {
                continue;
            }
            let interior = interior_limit(level, sharpness);
            let mbedge = (i32::from(level) + 2) * 2 + interior;
            let sub_bedge = i32::from(level) * 2 + interior;
            let hev_t = keyframe_hev_threshold(level);
            let do_interior = filter_interior[mb_y * mb_cols + mb_x];
            let (left, top) = (mb_x > 0, mb_y > 0);
            normal_filter_plane(
                y,
                y_stride,
                mb_x * 16,
                mb_y * 16,
                16,
                &[4, 8, 12],
                left,
                top,
                do_interior,
                mbedge,
                sub_bedge,
                interior,
                hev_t,
            );
            normal_filter_plane(
                u,
                c_stride,
                mb_x * 8,
                mb_y * 8,
                8,
                &[4],
                left,
                top,
                do_interior,
                mbedge,
                sub_bedge,
                interior,
                hev_t,
            );
            normal_filter_plane(
                v,
                c_stride,
                mb_x * 8,
                mb_y * 8,
                8,
                &[4],
                left,
                top,
                do_interior,
                mbedge,
                sub_bedge,
                interior,
                hev_t,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions_round_trip() {
        for v in 0u8..=255 {
            assert_eq!(s2u(u2s(v)), v);
        }
        assert_eq!(c(200), 127);
        assert_eq!(c(-200), -128);
    }

    #[test]
    fn common_adjust_shaves_a_small_edge_difference() {
        // A modest step P0=120, Q0=136 (with equal outer pixels) is brought closer together.
        let mut seg = [120u8, 120, 136, 136];
        common_adjust(true, &mut seg, 0, 1, 2, 3);
        assert!(
            seg[1] > 120 && seg[1] <= 128,
            "p0 rises toward the edge: {}",
            seg[1]
        );
        assert!(
            seg[2] < 136 && seg[2] >= 128,
            "q0 falls toward the edge: {}",
            seg[2]
        );
        assert!((i32::from(seg[1]) - 120).abs() <= 8);
    }

    #[test]
    fn simple_segment_ignores_edges_above_the_limit() {
        // A large step (P0=20, Q0=235) with a tiny limit must be left untouched.
        let mut seg = [20u8, 20, 235, 235];
        simple_segment(4, &mut seg, 0, 1, 2, 3);
        assert_eq!(seg, [20, 20, 235, 235]);
    }

    #[test]
    fn interior_limit_floors_at_one_and_applies_sharpness() {
        assert_eq!(interior_limit(0, 0), 1);
        assert_eq!(interior_limit(20, 0), 20);
        assert_eq!(interior_limit(20, 1), 8); // 20 >> 1 = 10, capped to 9 - 1 = 8
        assert_eq!(interior_limit(40, 6), 3); // 40 >> 2 = 10, capped to 9 - 6 = 3
    }

    #[test]
    fn zero_level_is_a_no_op() {
        let mut plane: Vec<u8> = (0..16 * 16).map(|i| (i % 256) as u8).collect();
        let before = plane.clone();
        simple_filter_luma(&mut plane, 16, 1, 1, &[0], 0, &[true]);
        assert_eq!(plane, before);
    }

    #[test]
    fn filters_the_macroblock_edge_but_skips_unflagged_interiors() {
        // Two macroblocks side by side (32x16): left half 100, right half 120.
        let stride = 32;
        let mut plane = vec![0u8; stride * 16];
        for r in 0..16 {
            for x in 0..32 {
                plane[r * stride + x] = if x < 16 { 100 } else { 120 };
            }
        }
        // filter_interior = false for both macroblocks → only the inter-macroblock edge at x=16 runs.
        simple_filter_luma(&mut plane, stride, 2, 1, &[20, 20], 0, &[false, false]);
        // The edge pixels move toward each other; far pixels and the (skipped) interior are untouched.
        assert_eq!((plane[15], plane[16]), (105, 115));
        assert_eq!(plane[0], 100, "left interior untouched");
        assert_eq!(plane[31], 120, "right interior untouched");
        assert_eq!(plane[3], 100, "interior subblock edge at x=4 skipped");
    }

    #[test]
    fn filter_yes_and_hev_gate_on_the_thresholds() {
        let smooth = [60u8, 64, 68, 72, 84, 88, 92, 96]; // p3 p2 p1 p0 | q0 q1 q2 q3
        assert!(filter_yes(100, 1000, &smooth, 0, 1, 2, 3, 4, 5, 6, 7));
        assert!(
            !filter_yes(2, 1000, &smooth, 0, 1, 2, 3, 4, 5, 6, 7),
            "tiny interior limit disables"
        );
        assert!(
            !hev(10, smooth[2], smooth[3], smooth[4], smooth[5]),
            "gentle ramp is not high-variance"
        );
        assert!(
            hev(10, 50, 100, 110, 160),
            "a 50-step on one side is high-variance"
        );
    }

    #[test]
    fn normal_mb_filter_adjusts_six_pixels_when_variance_is_low() {
        // A gentle ramp across the edge (no HEV): the six-tap filter touches p2..q2 but not p3/q3.
        let mut seg = [60u8, 64, 68, 72, 84, 88, 92, 96];
        let before = seg;
        normal_mb_segment(&mut seg, 4, 1, 1000, 100, 10);
        for i in [1usize, 2, 3, 4, 5, 6] {
            assert_ne!(seg[i], before[i], "pixel {i} adjusted");
        }
        assert_eq!(seg[0], before[0], "p3 untouched");
        assert_eq!(seg[7], before[7], "q3 untouched");
    }

    #[test]
    fn normal_mb_filter_falls_back_to_simple_under_high_variance() {
        // A 50-step before the edge triggers HEV → only the two-pixel simple adjust runs (p0, q0);
        // the outer p2/q2 are left untouched.
        let mut seg = [50u8, 50, 50, 100, 110, 160, 160, 160];
        let before = seg;
        normal_mb_segment(&mut seg, 4, 1, 10_000, 10_000, 10);
        assert_eq!(seg[1], before[1], "p2 untouched under HEV");
        assert_eq!(seg[6], before[6], "q2 untouched under HEV");
        assert_ne!(seg[3], before[3], "p0 adjusted");
        assert_ne!(seg[4], before[4], "q0 adjusted");
    }
}
