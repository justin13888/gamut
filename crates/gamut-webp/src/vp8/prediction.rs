//! VP8 intra prediction (RFC 6386 §11–§12) and key-frame mode coding. Key-frame intra only.
//!
//! Key frames code, per macroblock, a luma 16×16 mode and a chroma 8×8 mode, each tree-coded over the
//! boolean coder with fixed key-frame probabilities (§11.2, §11.4). The minimal keystone (P7) predicts
//! and signals only `DC_PRED`; the remaining whole-block modes (V/H/TM) and 4×4 `B_PRED` land in P8
//! and P9. The [`LumaMode`] / [`SubBlockMode`] / [`ChromaMode`] enums name the full mode space; the
//! `*_PRED` constants below are the same values as tree-leaf indices for the coders.

use super::bool_coder::{Prob, Tree};
use super::transform::clamp255;

/// Luma 16×16 prediction mode (RFC 6386 §11.2, §12.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LumaMode {
    /// DC (average of available top/left edges) prediction.
    Dc,
    /// Vertical prediction from the row above.
    Vertical,
    /// Horizontal prediction from the column to the left.
    Horizontal,
    /// TrueMotion prediction (top row + left column − top-left corner).
    TrueMotion,
    /// Per-4×4-subblock prediction; selects a [`SubBlockMode`] for each of the 16 subblocks.
    BPred,
}

/// Luma 4×4 subblock prediction mode, used when the macroblock mode is [`LumaMode::BPred`]
/// (RFC 6386 §11.2, §12.3). Ten directional / averaging modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubBlockMode {
    /// DC (average) prediction.
    Dc,
    /// TrueMotion prediction.
    TrueMotion,
    /// Vertical prediction.
    Vertical,
    /// Horizontal prediction.
    Horizontal,
    /// Down-left diagonal prediction.
    LeftDown,
    /// Down-right diagonal prediction.
    RightDown,
    /// Vertical-right diagonal prediction.
    VerticalRight,
    /// Vertical-left diagonal prediction.
    VerticalLeft,
    /// Horizontal-down diagonal prediction.
    HorizontalDown,
    /// Horizontal-up diagonal prediction.
    HorizontalUp,
}

/// Chroma 8×8 prediction mode (RFC 6386 §12.2). The same four modes as the luma 16×16 set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromaMode {
    /// DC (average) prediction.
    Dc,
    /// Vertical prediction.
    Vertical,
    /// Horizontal prediction.
    Horizontal,
    /// TrueMotion prediction.
    TrueMotion,
}

/// `DC_PRED` whole-block mode index (matches [`LumaMode::Dc`] / [`ChromaMode::Dc`]).
pub const DC_PRED: usize = 0;
/// `V_PRED` (vertical) whole-block mode index.
pub const V_PRED: usize = 1;
/// `H_PRED` (horizontal) whole-block mode index.
pub const H_PRED: usize = 2;
/// `TM_PRED` (TrueMotion) whole-block mode index.
pub const TM_PRED: usize = 3;
/// `B_PRED` (per-4×4-subblock luma) mode index.
pub const B_PRED: usize = 4;

/// Key-frame luma 16×16 mode tree (RFC 6386 §11.2 / §8.2 `kf_ymode_tree`). Leaf values are the mode
/// indices above (`-4` = `B_PRED`, `0` = `DC_PRED`, `-1` = `V_PRED`, …).
#[rustfmt::skip]
pub const KF_YMODE_TREE: &Tree = &[-4, 2, 4, 6, 0, -1, -2, -3];

/// Key-frame luma 16×16 mode probabilities (RFC 6386 §11.2 `kf_ymode_prob`).
pub const KF_YMODE_PROB: [Prob; 4] = [145, 156, 163, 128];

/// Chroma 8×8 mode tree (RFC 6386 §11.4 / §8.2 `uv_mode_tree`).
#[rustfmt::skip]
pub const KF_UV_MODE_TREE: &Tree = &[0, 2, -1, 4, -2, -3];

/// Key-frame chroma 8×8 mode probabilities (RFC 6386 §11.4 `kf_uv_mode_prob`).
pub const KF_UV_MODE_PROB: [Prob; 3] = [142, 114, 183];

/// Computes the single `DC_PRED` value filling an `n`×`n` block (RFC 6386 §12.2/§12.3): the rounded
/// average of the available reconstructed neighbors. `above` is the `n`-pixel row immediately above
/// the block and `left` the `n`-pixel column immediately to its left, each `None` when off-frame.
/// With neither neighbor (the top-left block) the value is the constant 128; with one, only that
/// neighbor is averaged (the spec's edge exception, *not* the 127/129 out-of-bounds fills).
#[must_use]
pub fn dc_predict(n: usize, above: Option<&[u8]>, left: Option<&[u8]>) -> u8 {
    let sum_of = |pixels: &[u8]| pixels.iter().map(|&p| u32::from(p)).sum::<u32>();
    let round_shift = |sum: u32, shf: u32| ((sum + (1 << (shf - 1))) >> shf) as u8;
    match (above, left) {
        // 2n summands -> shf = log2(2n) (5 for luma, 4 for chroma).
        (Some(a), Some(l)) => round_shift(sum_of(a) + sum_of(l), (2 * n).trailing_zeros()),
        // n summands -> shf = log2(n) (4 for luma, 3 for chroma).
        (Some(a), None) => round_shift(sum_of(a), n.trailing_zeros()),
        (None, Some(l)) => round_shift(sum_of(l), n.trailing_zeros()),
        (None, None) => 128,
    }
}

/// The `n` neighbor pixels, substituting `fill` for an off-frame edge.
fn edge(pixels: Option<&[u8]>, fill: u8) -> [u8; 16] {
    let mut e = [fill; 16];
    if let Some(p) = pixels {
        e[..p.len()].copy_from_slice(p);
    }
    e
}

/// Fills `out[..n*n]` (row-major) with the whole-block prediction for `mode` — `DC_PRED`, `V_PRED`,
/// `H_PRED`, or `TM_PRED` (RFC 6386 §12.2/§12.3). `above`/`left` are the `n` reconstructed neighbor
/// pixels (`None` off-frame: V/H/TM substitute the 127/129 out-of-bounds values, while DC uses its
/// averaging edge exception); `corner` is the above-left pixel TrueMotion propagates from.
pub fn predict_block(
    mode: usize,
    n: usize,
    above: Option<&[u8]>,
    left: Option<&[u8]>,
    corner: u8,
    out: &mut [u8],
) {
    match mode {
        V_PRED => {
            let a = edge(above, 127);
            for r in 0..n {
                out[r * n..r * n + n].copy_from_slice(&a[..n]);
            }
        }
        H_PRED => {
            let l = edge(left, 129);
            for r in 0..n {
                out[r * n..r * n + n].fill(l[r]);
            }
        }
        TM_PRED => {
            let a = edge(above, 127);
            let l = edge(left, 129);
            let p = i32::from(corner);
            for r in 0..n {
                for c in 0..n {
                    out[r * n + c] = clamp255(i32::from(l[r]) + i32::from(a[c]) - p);
                }
            }
        }
        _ => out[..n * n].fill(dc_predict(n, above, left)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dc_top_left_is_128() {
        assert_eq!(dc_predict(16, None, None), 128);
        assert_eq!(dc_predict(8, None, None), 128);
    }

    #[test]
    fn dc_single_edge_averages_that_edge() {
        let above = [100u8; 16];
        assert_eq!(dc_predict(16, Some(&above), None), 100);
        let left = [10u8, 10, 10, 10, 20, 20, 20, 20];
        let expected = ((10 * 4 + 20 * 4 + 4) >> 3) as u8;
        assert_eq!(dc_predict(8, None, Some(&left)), expected);
    }

    #[test]
    fn dc_both_edges_average_all() {
        let a = [64u8; 16];
        let l = [64u8; 16];
        assert_eq!(dc_predict(16, Some(&a), Some(&l)), 64);
        let a2 = [200u8; 16];
        let l2 = [0u8; 16];
        assert_eq!(
            dc_predict(16, Some(&a2), Some(&l2)),
            ((200 * 16 + 16) >> 5) as u8
        );
    }

    #[test]
    fn mode_trees_are_well_formed() {
        assert_eq!(KF_YMODE_TREE.len(), 8);
        assert_eq!(KF_UV_MODE_TREE.len(), 6);
        assert_eq!(LumaMode::Dc as usize, DC_PRED);
        assert_eq!(ChromaMode::TrueMotion as usize, TM_PRED);
    }

    #[test]
    fn vertical_prediction_copies_the_above_row() {
        let above: Vec<u8> = (0..16).map(|i| (i * 10) as u8).collect();
        let mut out = [0u8; 256];
        predict_block(V_PRED, 16, Some(&above), Some(&[50u8; 16]), 200, &mut out);
        for r in 0..16 {
            assert_eq!(&out[r * 16..r * 16 + 16], &above[..]);
        }
    }

    #[test]
    fn horizontal_prediction_copies_the_left_column() {
        let left: Vec<u8> = (0..16).map(|i| (i * 10) as u8).collect();
        let mut out = [0u8; 256];
        predict_block(H_PRED, 16, Some(&[50u8; 16]), Some(&left), 200, &mut out);
        for r in 0..16 {
            assert!(out[r * 16..r * 16 + 16].iter().all(|&p| p == left[r]));
        }
    }

    #[test]
    fn truemotion_propagates_from_the_corner() {
        // X[r][c] = clamp255(L[r] + A[c] - P).
        let above = [10u8, 20, 30, 40];
        let left = [100u8, 110, 120, 130];
        let p = 50i32;
        let mut out = [0u8; 16];
        predict_block(TM_PRED, 4, Some(&above), Some(&left), p as u8, &mut out);
        for r in 0..4 {
            for c in 0..4 {
                let expect = (i32::from(left[r]) + i32::from(above[c]) - p).clamp(0, 255) as u8;
                assert_eq!(out[r * 4 + c], expect, "TM at ({r},{c})");
            }
        }
    }

    #[test]
    fn off_frame_edges_use_127_and_129() {
        let mut out = [0u8; 256];
        predict_block(V_PRED, 16, None, Some(&[5u8; 16]), 0, &mut out);
        assert!(
            out.iter().all(|&p| p == 127),
            "vertical off the top row fills 127"
        );
        predict_block(H_PRED, 16, Some(&[5u8; 16]), None, 0, &mut out);
        assert!(
            out.iter().all(|&p| p == 129),
            "horizontal off the left column fills 129"
        );
    }
}
