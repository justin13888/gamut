//! VP8 in-loop deblocking filter (RFC 6386 §15).
//!
//! After reconstruction, VP8 smooths macroblock and subblock edges. Crucially the filter runs as a
//! **final whole-frame pass** — intra prediction (§12) reads the *unfiltered* reconstruction, so the
//! encoder reconstructs the frame unfiltered (predicting from it) and only then deblocks the output.
//!
//! This module implements the **simple filter** (§15.2), which acts on luma edges only. The normal
//! filter (§15.3) and its high-edge-variance test land in P11. Control parameters derive from the
//! frame's `loop_filter_level` and `sharpness_level` per §15.4. Tracked in `../STATUS.md` section M.

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
/// is `B_PRED` or carries coefficients). Chroma is left unfiltered. A zero `level` is a no-op.
pub fn simple_filter_luma(
    plane: &mut [u8],
    stride: usize,
    mb_cols: usize,
    mb_rows: usize,
    level: u8,
    sharpness: u8,
    filter_interior: &[bool],
) {
    if level == 0 {
        return;
    }
    let interior = interior_limit(level, sharpness);
    let mbedge_limit = (i32::from(level) + 2) * 2 + interior;
    let sub_bedge_limit = i32::from(level) * 2 + interior;
    for mb_y in 0..mb_rows {
        for mb_x in 0..mb_cols {
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
        simple_filter_luma(&mut plane, 16, 1, 1, 0, 0, &[true]);
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
        simple_filter_luma(&mut plane, stride, 2, 1, 20, 0, &[false, false]);
        // The edge pixels move toward each other; far pixels and the (skipped) interior are untouched.
        assert_eq!((plane[15], plane[16]), (105, 115));
        assert_eq!(plane[0], 100, "left interior untouched");
        assert_eq!(plane[31], 120, "right interior untouched");
        assert_eq!(plane[3], 100, "interior subblock edge at x=4 skipped");
    }
}
