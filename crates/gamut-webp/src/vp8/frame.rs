//! VP8 key-frame reconstruction pipeline (RFC 6386 §10–§14): the macroblock loop that ties together
//! prediction, the transforms, quantization, and token coding into an encodable/decodable frame.
//!
//! This is the keystone of the lossy path. Each macroblock is predicted from the **reconstructed**
//! neighbors in a recon buffer (the encoder predicts exactly as the decoder does), so the encoder's
//! reconstruction is bit-identical to any conformant decoder's output. Luma uses whole-block 16×16
//! DC/V/H/TM **or** per-4×4 `B_PRED` (ten directional submodes), and chroma whole-block 8×8 DC/V/H/TM;
//! the encoder picks the lowest-SAD candidate per macroblock. A whole-block macroblock carries a Y2
//! (luma-DC WHT) block; a `B_PRED` one codes luma DC inline (plane 3). The reconstruction is deblocked
//! by the simple or normal loop filter as a final pass. Tokens may be split across 1/2/4/8 partitions
//! by macroblock row, and all-zero macroblocks are coded as skipped. Per-macroblock loop-filter
//! adjustments are the remaining VP8 header feature. STATUS.md section L.

// The macroblock/block math indexes several fixed-size arrays in lock-step (and over partial ranges
// like `1..16`), where explicit indices read closer to the spec than iterator adaptors.
#![allow(clippy::needless_range_loop)]

use gamut_color::Yuv420;
use gamut_core::{Error, Result};

use super::bool_coder::{BoolDecoder, BoolEncoder};
use super::header::{
    self, LoopFilterParams, QuantIndices, Segmentation, UNCOMPRESSED_CHUNK_LEN, Vp8FrameHeader,
};
use super::loop_filter;
use super::prediction::{self, B_DC_PRED, B_PRED, DC_PRED, H_PRED, NUM_BMODES, TM_PRED, V_PRED};
use super::quant::{self, QuantFactors};
use super::tokens::{self, CoeffProbs};
use super::transform::{clamp255, fdct4x4, fwht4x4, idct4x4, iwht4x4};

/// The whole-block prediction modes the encoder considers, in signaling order.
const WHOLE_BLOCK_MODES: [usize; 4] = [DC_PRED, V_PRED, H_PRED, TM_PRED];

/// SAD margin by which per-subblock `B_PRED` must beat the best whole-block mode to be chosen — a
/// coarse stand-in for `B_PRED`'s extra mode-signaling cost (true rate-distortion search is issue #32).
const BPRED_SAD_PENALTY: u32 = 160;

/// Segment-id coding tree (RFC 6386 §10 `mb_segment_tree`): four leaves over two boolean decisions.
const MB_SEGMENT_TREE: &[i8] = &[2, 4, 0, -1, -2, -3];

/// Per-segment quantizer deltas the encoder assigns (delta mode) when segmentation is enabled — a
/// coarse spread so distinct macroblock regions get distinct quantizers (refinement is issue #32).
const SEGMENT_QUANT_DELTAS: [i8; 4] = [-12, -4, 4, 12];

/// Encoder feature toggles for a frame. Defaults to the normal loop filter, no segmentation, and a
/// single token partition.
#[derive(Clone, Copy)]
pub struct EncodeOptions {
    /// Use the simple loop filter instead of the normal one.
    pub simple_filter: bool,
    /// Emit four quantizer segments, assigned per macroblock by luma mean.
    pub segmented: bool,
    /// Number of DCT token partitions (1, 2, 4, or 8); macroblock rows are assigned round-robin.
    pub partitions: u8,
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            simple_filter: false,
            segmented: false,
            partitions: 1,
        }
    }
}

/// The clamped base quantizer index for segment `s` (RFC 6386 §9.3/§10): the absolute or
/// delta-adjusted value when segmentation is enabled, else the frame base.
fn segment_q_index(seg: &Segmentation, base_y_ac: u8, s: usize) -> i32 {
    if !seg.enabled {
        return i32::from(base_y_ac);
    }
    let q = if seg.abs_delta {
        i32::from(seg.quantizer[s])
    } else {
        i32::from(base_y_ac) + i32::from(seg.quantizer[s])
    };
    q.clamp(0, 127)
}

/// The four per-segment quantizer factor sets for a frame (all equal when segmentation is disabled).
fn segment_quant_factors(header: &Vp8FrameHeader) -> [QuantFactors; 4] {
    core::array::from_fn(|s| {
        let base_q = segment_q_index(&header.segmentation, header.quant.y_ac, s);
        QuantFactors::new(base_q, &header.quant)
    })
}

/// The mean luma of macroblock `(mb_x, mb_y)` in a `stride`-wide plane, used to assign its segment.
fn mb_luma_mean(src: &[u8], stride: usize, mb_x: usize, mb_y: usize) -> u32 {
    let (px, py) = (mb_x * 16, mb_y * 16);
    let mut sum = 0u32;
    for r in 0..16 {
        for c in 0..16 {
            sum += u32::from(src[(py + r) * stride + px + c]);
        }
    }
    sum / 256
}

/// Per-macroblock-column entropy context: whether the prior block in each position carried at least
/// one non-zero coefficient (RFC 6386 §13.3). A single instance also serves as the running "left"
/// context, reset at the start of each macroblock row.
#[derive(Clone, Copy, Default)]
struct EntropyCtx {
    /// Y2 (luma-DC WHT) block.
    y2: bool,
    /// The four luma sub-block columns (above) / rows (left).
    y: [bool; 4],
    /// The two U sub-block columns / rows.
    u: [bool; 2],
    /// The two V sub-block columns / rows.
    v: [bool; 2],
}

/// One macroblock's quantized coefficient levels: the Y2 block, 16 luma sub-blocks, 4 U and 4 V.
#[derive(Clone, Default)]
struct MbLevels {
    y2: [i16; 16],
    y: [[i16; 16]; 16],
    u: [[i16; 16]; 4],
    v: [[i16; 16]; 4],
}

/// Macroblock-aligned reconstructed YUV planes (luma `mb_cols*16 × mb_rows*16`, chroma half each).
pub struct FrameBuffers {
    width: u32,
    height: u32,
    mb_cols: usize,
    mb_rows: usize,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

impl FrameBuffers {
    fn new(width: u32, height: u32) -> Self {
        let mb_cols = (width as usize).div_ceil(16);
        let mb_rows = (height as usize).div_ceil(16);
        Self {
            width,
            height,
            mb_cols,
            mb_rows,
            y: vec![0u8; mb_cols * 16 * mb_rows * 16],
            u: vec![0u8; mb_cols * 8 * mb_rows * 8],
            v: vec![0u8; mb_cols * 8 * mb_rows * 8],
        }
    }

    fn y_stride(&self) -> usize {
        self.mb_cols * 16
    }

    fn c_stride(&self) -> usize {
        self.mb_cols * 8
    }

    /// Crops the reconstruction to a visible-resolution [`Yuv420`].
    #[must_use]
    pub fn to_yuv420(&self) -> Yuv420 {
        let (w, h) = (self.width as usize, self.height as usize);
        let (cw, ch) = (
            Yuv420::chroma_width(self.width) as usize,
            Yuv420::chroma_height(self.height) as usize,
        );
        let crop = |plane: &[u8], stride: usize, pw: usize, ph: usize| {
            let mut out = vec![0u8; pw * ph];
            for row in 0..ph {
                out[row * pw..row * pw + pw]
                    .copy_from_slice(&plane[row * stride..row * stride + pw]);
            }
            out
        };
        let y = crop(&self.y, self.y_stride(), w, h);
        let u = crop(&self.u, self.c_stride(), cw, ch);
        let v = crop(&self.v, self.c_stride(), cw, ch);
        Yuv420::new(self.width, self.height, y, u, v).expect("cropped planes match dimensions")
    }
}

/// Picks a loop-filter strength from the base quantizer — stronger quantization deblocks harder. A
/// coarse heuristic (true filter-level selection is part of issue #32); a level of 0 disables it.
fn filter_level(quant_index: u8) -> u8 {
    quant_index / 2
}

/// Applies the frame's configured loop filter to the reconstruction as a final whole-frame pass: the
/// simple filter deblocks luma only, the normal filter luma and chroma. A zero level is a no-op.
fn apply_loop_filter(recon: &mut FrameBuffers, lf: &LoopFilterParams, filter_interior: &[bool]) {
    if lf.level == 0 {
        return;
    }
    let (ys, cs, mbc, mbr) = (
        recon.y_stride(),
        recon.c_stride(),
        recon.mb_cols,
        recon.mb_rows,
    );
    if lf.simple {
        loop_filter::simple_filter_luma(
            &mut recon.y,
            ys,
            mbc,
            mbr,
            lf.level,
            lf.sharpness,
            filter_interior,
        );
    } else {
        loop_filter::normal_filter(
            &mut recon.y,
            &mut recon.u,
            &mut recon.v,
            ys,
            cs,
            mbc,
            mbr,
            lf.level,
            lf.sharpness,
            filter_interior,
        );
    }
}

/// Whether a macroblock carries any non-zero quantized coefficient — the second half of the
/// loop-filter interior-edge skip rule (RFC 6386 §15.1).
fn mb_has_coeffs(levels: &MbLevels) -> bool {
    levels.y2.iter().any(|&x| x != 0)
        || levels.y.iter().flatten().any(|&x| x != 0)
        || levels.u.iter().flatten().any(|&x| x != 0)
        || levels.v.iter().flatten().any(|&x| x != 0)
}

/// Builds the minimal key-frame header for the given dimensions, base quantizer, and filter type.
fn frame_header(width: u32, height: u32, quant_index: u8, simple_filter: bool) -> Vp8FrameHeader {
    Vp8FrameHeader {
        width: width as u16,
        height: height as u16,
        horizontal_scale: 0,
        vertical_scale: 0,
        version: 0,
        color_space: 0,
        clamp_required: true,
        segmentation: Segmentation::default(),
        loop_filter: LoopFilterParams {
            simple: simple_filter,
            level: filter_level(quant_index),
            sharpness: 0,
        },
        token_partitions: 1,
        quant: QuantIndices {
            y_ac: quant_index,
            ..QuantIndices::default()
        },
        refresh_entropy_probs: true,
        // Enable per-macroblock skip coding. The skip-false probability falls with the quantizer,
        // since coarser quantization yields more all-zero (skippable) macroblocks.
        mb_no_skip_coeff: true,
        prob_skip_false: (255 - quant_index).max(1),
    }
}

/// Resets a macroblock's coefficient context to "no non-zero coefficients" for a skipped macroblock
/// (RFC 6386 §11.1): equivalent to coding all-zero blocks, but the `B_PRED` Y2 context persists since
/// such a macroblock carries no Y2 block.
fn clear_mb_context(above: &mut EntropyCtx, left: &mut EntropyCtx, is_bpred: bool) {
    if !is_bpred {
        above.y2 = false;
        left.y2 = false;
    }
    above.y = [false; 4];
    left.y = [false; 4];
    above.u = [false; 2];
    left.u = [false; 2];
    above.v = [false; 2];
    left.v = [false; 2];
}

/// Reconstructs a skipped `B_PRED` macroblock's luma: each subblock is its prediction with no residual
/// (the encoder's all-zero-coefficient reconstruction).
fn reconstruct_bpred_zero(
    recon: &mut FrameBuffers,
    mb_x: usize,
    mb_y: usize,
    sub_modes: &[usize; 16],
    above_right: &[u8; 4],
) {
    let (px, py, rstride) = (mb_x * 16, mb_y * 16, recon.y_stride());
    for i in 0..16 {
        let (r, c) = (i / 4, i % 4);
        let (sx, sy) = (px + c * 4, py + r * 4);
        let (a, l, corner) = subblock_neighbors(recon, sx, sy, c, above_right);
        let pred = prediction::subblock_predict(sub_modes[i], &a, &l, corner);
        let pred_i16: [i16; 16] = core::array::from_fn(|k| i16::from(pred[k]));
        write_block(&mut recon.y, rstride, sx, sy, &pred_i16, &[0i16; 16]);
    }
}

/// Replicates `src` (`sw × sh`) into a `dw × dh` plane, extending the right and bottom edges.
fn pad_plane(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    let mut dst = vec![0u8; dw * dh];
    for y in 0..dh {
        let sy = y.min(sh - 1);
        for x in 0..dw {
            dst[y * dw + x] = src[sy * sw + x.min(sw - 1)];
        }
    }
    dst
}

/// Gathers the `n`-pixel row at `(x, y)` of `plane` into a fixed buffer (only `[..n]` is meaningful).
fn row_at(plane: &[u8], stride: usize, x: usize, y: usize, n: usize) -> [u8; 16] {
    let mut b = [0u8; 16];
    b[..n].copy_from_slice(&plane[y * stride + x..y * stride + x + n]);
    b
}

/// Gathers the `n`-pixel column at `(x, y)` of `plane` into a fixed buffer.
fn col_at(plane: &[u8], stride: usize, x: usize, y: usize, n: usize) -> [u8; 16] {
    let mut b = [0u8; 16];
    for (r, slot) in b[..n].iter_mut().enumerate() {
        *slot = plane[(y + r) * stride + x];
    }
    b
}

/// Reads a 4×4 block at `(x, y)` of `plane` as 16-bit samples.
fn read_block(plane: &[u8], stride: usize, x: usize, y: usize) -> [i16; 16] {
    let mut b = [0i16; 16];
    for r in 0..4 {
        for c in 0..4 {
            b[r * 4 + c] = i16::from(plane[(y + r) * stride + x + c]);
        }
    }
    b
}

/// Extracts the 4×4 sub-block at `(sub_x, sub_y)` of a `stride`-wide prediction block, as 16-bit.
fn sub_pred(pred: &[u8], stride: usize, sub_x: usize, sub_y: usize) -> [i16; 16] {
    let mut out = [0i16; 16];
    for r in 0..4 {
        for c in 0..4 {
            out[r * 4 + c] = i16::from(pred[(sub_y + r) * stride + sub_x + c]);
        }
    }
    out
}

/// Writes `clamp255(pred + residue)` into the 4×4 block at `(x, y)` of `plane`.
fn write_block(
    plane: &mut [u8],
    stride: usize,
    x: usize,
    y: usize,
    pred: &[i16; 16],
    residue: &[i16; 16],
) {
    for r in 0..4 {
        for c in 0..4 {
            let v = i32::from(pred[r * 4 + c]) + i32::from(residue[r * 4 + c]);
            plane[(y + r) * stride + x + c] = clamp255(v);
        }
    }
}

/// The above-left corner pixel for prediction: 127 on the top macroblock row, 129 on the left column,
/// otherwise the reconstructed pixel (RFC 6386 §12.2).
fn corner_pixel(plane: &[u8], stride: usize, px: usize, py: usize, mb_x: usize, mb_y: usize) -> u8 {
    if mb_y == 0 {
        127
    } else if mb_x == 0 {
        129
    } else {
        plane[(py - 1) * stride + px - 1]
    }
}

/// One reconstructed luma pixel, or its off-frame edge value (127 above the frame, 129 to the left).
fn luma_pixel(recon: &FrameBuffers, y: i32, x: i32) -> u8 {
    if y < 0 {
        127
    } else if x < 0 {
        129
    } else {
        recon.y[y as usize * recon.y_stride() + x as usize]
    }
}

/// The four above-right pixels of the macroblock's top-right subblock, shared by all right-column
/// subblocks (RFC 6386 §12.3 `copy_down`). Matching libwebp: 127 on the top row; the next
/// macroblock's top-left four pixels normally; or the current macroblock's last above pixel
/// replicated on the rightmost column (`frame_dec.c`: `memset(top_right, top[15])`).
fn above_right_source(recon: &FrameBuffers, mb_x: usize, mb_y: usize) -> [u8; 4] {
    if mb_y == 0 {
        return [127; 4];
    }
    let stride = recon.y_stride();
    let row = (mb_y * 16 - 1) * stride;
    if mb_x + 1 >= recon.mb_cols {
        [recon.y[row + mb_x * 16 + 15]; 4]
    } else {
        let base = row + mb_x * 16 + 16;
        [
            recon.y[base],
            recon.y[base + 1],
            recon.y[base + 2],
            recon.y[base + 3],
        ]
    }
}

/// Gathers a 4×4 luma subblock's prediction neighbors from the in-place reconstruction: the eight
/// above pixels `A[0..8]` (four above, four above-right), the four left `L[0..4]`, and the above-left
/// corner. `(sx, sy)` is the subblock's top-left in frame coordinates and `c` its column within the
/// macroblock (the right column, `c == 3`, takes its above-right from the shared `above_right`).
fn subblock_neighbors(
    recon: &FrameBuffers,
    sx: usize,
    sy: usize,
    c: usize,
    above_right: &[u8; 4],
) -> ([u8; 8], [u8; 4], u8) {
    let (xi, yi) = (sx as i32, sy as i32);
    let corner = luma_pixel(recon, yi - 1, xi - 1);
    let mut a = [0u8; 8];
    for k in 0..4 {
        a[k] = luma_pixel(recon, yi - 1, xi + k as i32);
    }
    if c == 3 {
        a[4..8].copy_from_slice(above_right);
    } else {
        for k in 0..4 {
            a[4 + k] = luma_pixel(recon, yi - 1, xi + 4 + k as i32);
        }
    }
    let mut l = [0u8; 4];
    for k in 0..4 {
        l[k] = luma_pixel(recon, yi + k as i32, xi - 1);
    }
    (a, l, corner)
}

/// Produces the 16×16 luma prediction for macroblock `(mb_x, mb_y)` under whole-block `mode`.
fn predict_luma(recon: &FrameBuffers, mb_x: usize, mb_y: usize, mode: usize) -> [u8; 256] {
    let (px, py, stride) = (mb_x * 16, mb_y * 16, recon.y_stride());
    let above = (mb_y > 0).then(|| row_at(&recon.y, stride, px, py - 1, 16));
    let left = (mb_x > 0).then(|| col_at(&recon.y, stride, px - 1, py, 16));
    let corner = corner_pixel(&recon.y, stride, px, py, mb_x, mb_y);
    let mut pred = [0u8; 256];
    prediction::predict_block(
        mode,
        16,
        above.as_ref().map(|a| &a[..16]),
        left.as_ref().map(|l| &l[..16]),
        corner,
        &mut pred,
    );
    pred
}

/// Produces the 8×8 prediction for one chroma plane under whole-block `mode`.
fn predict_chroma(plane: &[u8], stride: usize, mb_x: usize, mb_y: usize, mode: usize) -> [u8; 64] {
    let (px, py) = (mb_x * 8, mb_y * 8);
    let above = (mb_y > 0).then(|| row_at(plane, stride, px, py - 1, 8));
    let left = (mb_x > 0).then(|| col_at(plane, stride, px - 1, py, 8));
    let corner = corner_pixel(plane, stride, px, py, mb_x, mb_y);
    let mut pred = [0u8; 64];
    prediction::predict_block(
        mode,
        8,
        above.as_ref().map(|a| &a[..8]),
        left.as_ref().map(|l| &l[..8]),
        corner,
        &mut pred,
    );
    pred
}

/// Sum of absolute differences between an `n`×`n` prediction and the source macroblock.
fn block_sad(pred: &[u8], src: &[u8], stride: usize, mb_x: usize, mb_y: usize, n: usize) -> u32 {
    let mut sad = 0u32;
    for r in 0..n {
        for c in 0..n {
            let s = i32::from(src[(mb_y * n + r) * stride + mb_x * n + c]);
            sad += s.abs_diff(i32::from(pred[r * n + c]));
        }
    }
    sad
}

/// Selects the lowest-SAD whole-block luma mode (a simple proxy; rate-distortion search is issue #32).
fn select_luma_mode(
    recon: &FrameBuffers,
    src: &[u8],
    stride: usize,
    mb_x: usize,
    mb_y: usize,
) -> usize {
    let mut best = (DC_PRED, u32::MAX);
    for mode in WHOLE_BLOCK_MODES {
        let sad = block_sad(
            &predict_luma(recon, mb_x, mb_y, mode),
            src,
            stride,
            mb_x,
            mb_y,
            16,
        );
        if sad < best.1 {
            best = (mode, sad);
        }
    }
    best.0
}

/// Selects the lowest-combined-SAD chroma mode (shared by U and V).
fn select_chroma_mode(
    recon: &FrameBuffers,
    src_u: &[u8],
    src_v: &[u8],
    stride: usize,
    mb_x: usize,
    mb_y: usize,
) -> usize {
    let mut best = (DC_PRED, u32::MAX);
    for mode in WHOLE_BLOCK_MODES {
        let su = block_sad(
            &predict_chroma(&recon.u, recon.c_stride(), mb_x, mb_y, mode),
            src_u,
            stride,
            mb_x,
            mb_y,
            8,
        );
        let sv = block_sad(
            &predict_chroma(&recon.v, recon.c_stride(), mb_x, mb_y, mode),
            src_v,
            stride,
            mb_x,
            mb_y,
            8,
        );
        if su + sv < best.1 {
            best = (mode, su + sv);
        }
    }
    best.0
}

/// Reconstructs the 16 luma sub-blocks of a macroblock: the Y2 inverse-WHT supplies each sub-block's
/// DC, the AC levels are dequantized, and `pred + idct` is written into the recon buffer. Shared by
/// the encoder and decoder.
fn reconstruct_luma(
    recon: &mut FrameBuffers,
    mb_x: usize,
    mb_y: usize,
    pred: &[u8; 256],
    levels: &MbLevels,
    qf: &QuantFactors,
) {
    let mut y2_dq = [0i16; 16];
    y2_dq[0] = quant::dequantize(levels.y2[0], qf.y2_dc);
    for k in 1..16 {
        y2_dq[k] = quant::dequantize(levels.y2[k], qf.y2_ac);
    }
    let dc = iwht4x4(&y2_dq);

    let stride = recon.y_stride();
    for i in 0..16 {
        let mut dq = [0i16; 16];
        dq[0] = dc[i];
        for k in 1..16 {
            dq[k] = quant::dequantize(levels.y[i][k], qf.y1_ac);
        }
        let residue = idct4x4(&dq);
        let (sc, sr) = (i % 4, i / 4);
        write_block(
            &mut recon.y,
            stride,
            mb_x * 16 + sc * 4,
            mb_y * 16 + sr * 4,
            &sub_pred(pred, 16, sc * 4, sr * 4),
            &residue,
        );
    }
}

/// Reconstructs the four sub-blocks of one chroma plane from full (DC+AC) levels.
fn reconstruct_chroma(
    plane: &mut [u8],
    stride: usize,
    mb_x: usize,
    mb_y: usize,
    pred: &[u8; 64],
    levels: &[[i16; 16]; 4],
    qf: &QuantFactors,
) {
    for i in 0..4 {
        let mut dq = [0i16; 16];
        dq[0] = quant::dequantize(levels[i][0], qf.uv_dc);
        for k in 1..16 {
            dq[k] = quant::dequantize(levels[i][k], qf.uv_ac);
        }
        let residue = idct4x4(&dq);
        let (sc, sr) = (i % 2, i / 2);
        write_block(
            plane,
            stride,
            mb_x * 8 + sc * 4,
            mb_y * 8 + sr * 4,
            &sub_pred(pred, 8, sc * 4, sr * 4),
            &residue,
        );
    }
}

/// Transforms + quantizes one luma macroblock against its prediction, returning the Y2 and per
/// sub-block AC levels.
fn quantize_luma(
    src: &[u8],
    stride: usize,
    mb_x: usize,
    mb_y: usize,
    pred: &[u8; 256],
    qf: &QuantFactors,
    levels: &mut MbLevels,
) {
    let mut y_coeffs = [[0i16; 16]; 16];
    let mut y_dc = [0i16; 16];
    for i in 0..16 {
        let (sc, sr) = (i % 4, i / 4);
        let block = read_block(src, stride, mb_x * 16 + sc * 4, mb_y * 16 + sr * 4);
        let p = sub_pred(pred, 16, sc * 4, sr * 4);
        let residue: [i16; 16] = core::array::from_fn(|k| block[k] - p[k]);
        y_coeffs[i] = fdct4x4(&residue);
        y_dc[i] = y_coeffs[i][0];
    }
    let y2_coeffs = fwht4x4(&y_dc);
    levels.y2[0] = quant::quantize(y2_coeffs[0], qf.y2_dc);
    for k in 1..16 {
        levels.y2[k] = quant::quantize(y2_coeffs[k], qf.y2_ac);
    }
    for i in 0..16 {
        for k in 1..16 {
            levels.y[i][k] = quant::quantize(y_coeffs[i][k], qf.y1_ac);
        }
    }
}

/// Transforms + quantizes one chroma plane's four sub-blocks against its prediction.
fn quantize_chroma(
    src: &[u8],
    stride: usize,
    mb_x: usize,
    mb_y: usize,
    pred: &[u8; 64],
    qf: &QuantFactors,
) -> [[i16; 16]; 4] {
    let mut levels = [[0i16; 16]; 4];
    for i in 0..4 {
        let (sc, sr) = (i % 2, i / 2);
        let block = read_block(src, stride, mb_x * 8 + sc * 4, mb_y * 8 + sr * 4);
        let p = sub_pred(pred, 8, sc * 4, sr * 4);
        let residue: [i16; 16] = core::array::from_fn(|k| block[k] - p[k]);
        let coeffs = fdct4x4(&residue);
        levels[i][0] = quant::quantize(coeffs[0], qf.uv_dc);
        for k in 1..16 {
            levels[i][k] = quant::quantize(coeffs[k], qf.uv_ac);
        }
    }
    levels
}

/// Encodes the luma plane of a `B_PRED` macroblock: per subblock (raster order), selects the
/// lowest-SAD submode, quantizes the residual (plane 3 — DC included, no Y2), and reconstructs in
/// place so the next subblock predicts from it. Returns the 16 submodes, their quantized levels, and
/// the total prediction SAD (for the macroblock mode decision).
fn encode_bpred_luma(
    recon: &mut FrameBuffers,
    src: &[u8],
    stride: usize,
    mb_x: usize,
    mb_y: usize,
    qf: &QuantFactors,
    above_right: &[u8; 4],
) -> ([usize; 16], [[i16; 16]; 16], u32) {
    let (px, py, rstride) = (mb_x * 16, mb_y * 16, recon.y_stride());
    let mut sub_modes = [B_DC_PRED; 16];
    let mut levels = [[0i16; 16]; 16];
    let mut total_sad = 0u32;
    for i in 0..16 {
        let (r, c) = (i / 4, i % 4);
        let (sx, sy) = (px + c * 4, py + r * 4);
        let (a, l, corner) = subblock_neighbors(recon, sx, sy, c, above_right);
        let src_sub = read_block(src, stride, sx, sy);
        let mut best = (B_DC_PRED, u32::MAX, [0u8; 16]);
        for m in 0..NUM_BMODES {
            let pred = prediction::subblock_predict(m, &a, &l, corner);
            let sad: u32 = (0..16)
                .map(|k| i32::from(src_sub[k]).abs_diff(i32::from(pred[k])))
                .sum();
            if sad < best.1 {
                best = (m, sad, pred);
            }
        }
        let (mode, sad, pred) = best;
        sub_modes[i] = mode;
        total_sad += sad;

        let residue: [i16; 16] = core::array::from_fn(|k| src_sub[k] - i16::from(pred[k]));
        let coeffs = fdct4x4(&residue);
        levels[i][0] = quant::quantize(coeffs[0], qf.y1_dc);
        for k in 1..16 {
            levels[i][k] = quant::quantize(coeffs[k], qf.y1_ac);
        }
        let mut dq = [0i16; 16];
        dq[0] = quant::dequantize(levels[i][0], qf.y1_dc);
        for k in 1..16 {
            dq[k] = quant::dequantize(levels[i][k], qf.y1_ac);
        }
        let residue = idct4x4(&dq);
        let pred_i16: [i16; 16] = core::array::from_fn(|k| i16::from(pred[k]));
        write_block(&mut recon.y, rstride, sx, sy, &pred_i16, &residue);
    }
    (sub_modes, levels, total_sad)
}

/// Decodes and reconstructs the luma plane of a `B_PRED` macroblock from its submodes and the token
/// partition, interleaving token decode and reconstruction (each subblock predicts from the one
/// before it) and threading the plane-3 non-zero context. Leaves the Y2 context untouched.
#[allow(clippy::too_many_arguments)] // the reconstruction loop genuinely needs all of this state
fn decode_bpred_luma(
    recon: &mut FrameBuffers,
    dec: &mut BoolDecoder,
    above: &mut EntropyCtx,
    left: &mut EntropyCtx,
    probs: &CoeffProbs,
    mb_x: usize,
    mb_y: usize,
    qf: &QuantFactors,
    sub_modes: &[usize; 16],
    above_right: &[u8; 4],
) {
    let (px, py, rstride) = (mb_x * 16, mb_y * 16, recon.y_stride());
    for i in 0..16 {
        let (r, c) = (i / 4, i % 4);
        let (sx, sy) = (px + c * 4, py + r * 4);
        let ctx = usize::from(above.y[c]) + usize::from(left.y[r]);
        let mut lev = [0i16; 16];
        let has = tokens::decode_block(dec, &mut lev, 3, ctx, probs);
        above.y[c] = has;
        left.y[r] = has;

        let (a, l, corner) = subblock_neighbors(recon, sx, sy, c, above_right);
        let pred = prediction::subblock_predict(sub_modes[i], &a, &l, corner);
        let mut dq = [0i16; 16];
        dq[0] = quant::dequantize(lev[0], qf.y1_dc);
        for k in 1..16 {
            dq[k] = quant::dequantize(lev[k], qf.y1_ac);
        }
        let residue = idct4x4(&dq);
        let pred_i16: [i16; 16] = core::array::from_fn(|k| i16::from(pred[k]));
        write_block(&mut recon.y, rstride, sx, sy, &pred_i16, &residue);
    }
}

/// The above/left subblock-mode context for the `j`th subblock (RFC 6386 §11.3): the mode of the
/// subblock above (within the macroblock for rows > 0, else `above_col`) and to the left (within for
/// columns > 0, else `left_col`).
fn bmode_context(
    sub_modes: &[usize; 16],
    above_col: &[usize; 4],
    left_col: &[usize; 4],
    i: usize,
) -> (usize, usize) {
    let (r, c) = (i / 4, i % 4);
    let a = if r > 0 {
        sub_modes[i - 4]
    } else {
        above_col[c]
    };
    let l = if c > 0 { sub_modes[i - 1] } else { left_col[r] };
    (a, l)
}

/// Writes the 16 `B_PRED` submodes, each tree-coded with its neighbor context (RFC 6386 §11.3).
fn write_bmodes(
    modes: &mut BoolEncoder,
    sub_modes: &[usize; 16],
    above_col: &[usize; 4],
    left_col: &[usize; 4],
) {
    for i in 0..16 {
        let (a, l) = bmode_context(sub_modes, above_col, left_col, i);
        modes.put_tree(
            prediction::BMODE_TREE,
            &prediction::KF_BMODE_PROB[a][l],
            sub_modes[i],
        );
    }
}

/// Reads the 16 `B_PRED` submodes, mirroring [`write_bmodes`].
fn read_bmodes(
    modes: &mut BoolDecoder,
    above_col: &[usize; 4],
    left_col: &[usize; 4],
) -> [usize; 16] {
    let mut sub_modes = [B_DC_PRED; 16];
    for i in 0..16 {
        let (a, l) = bmode_context(&sub_modes, above_col, left_col, i);
        sub_modes[i] = modes.get_tree(prediction::BMODE_TREE, &prediction::KF_BMODE_PROB[a][l]);
    }
    sub_modes
}

/// The macroblock's bottom-row and right-column subblock modes, to seed the above/left context of the
/// next row/column (RFC 6386 §11.3 caveat 4): the actual submodes for `B_PRED`, else the constant
/// derived from the whole-block luma mode.
fn bmode_propagation(
    is_bpred: bool,
    luma_mode: usize,
    sub_modes: &[usize; 16],
) -> ([usize; 4], [usize; 4]) {
    if is_bpred {
        (
            [sub_modes[12], sub_modes[13], sub_modes[14], sub_modes[15]],
            [sub_modes[3], sub_modes[7], sub_modes[11], sub_modes[15]],
        )
    } else {
        let bm = prediction::bmode_for_luma(luma_mode);
        ([bm; 4], [bm; 4])
    }
}

/// Codes one macroblock's coefficient blocks in Y2 → Y → U → V order, threading the `above`/`left`
/// non-zero context (RFC 6386 §13.3). A `B_PRED` macroblock has no Y2 block (its context persists)
/// and codes luma with plane 3 (DC included); otherwise luma uses plane 0 (DC carried by Y2).
fn encode_mb_tokens(
    enc: &mut BoolEncoder,
    above: &mut EntropyCtx,
    left: &mut EntropyCtx,
    probs: &CoeffProbs,
    levels: &MbLevels,
    is_bpred: bool,
) {
    if !is_bpred {
        let ctx = usize::from(above.y2) + usize::from(left.y2);
        let has = tokens::encode_block(enc, &levels.y2, 1, ctx, probs);
        above.y2 = has;
        left.y2 = has;
    }
    let plane = if is_bpred { 3 } else { 0 };
    for i in 0..16 {
        let (r, c) = (i / 4, i % 4);
        let ctx = usize::from(above.y[c]) + usize::from(left.y[r]);
        let has = tokens::encode_block(enc, &levels.y[i], plane, ctx, probs);
        above.y[c] = has;
        left.y[r] = has;
    }
    encode_chroma_tokens(enc, above, left, probs, levels);
}

/// Codes a macroblock's U then V chroma blocks (plane 2).
fn encode_chroma_tokens(
    enc: &mut BoolEncoder,
    above: &mut EntropyCtx,
    left: &mut EntropyCtx,
    probs: &CoeffProbs,
    levels: &MbLevels,
) {
    for (plane_levels, above_ctx, left_ctx) in [
        (&levels.u, &mut above.u, &mut left.u),
        (&levels.v, &mut above.v, &mut left.v),
    ] {
        for i in 0..4 {
            let (r, c) = (i / 2, i % 2);
            let ctx = usize::from(above_ctx[c]) + usize::from(left_ctx[r]);
            let has = tokens::encode_block(enc, &plane_levels[i], 2, ctx, probs);
            above_ctx[c] = has;
            left_ctx[r] = has;
        }
    }
}

/// Decodes a macroblock's U then V chroma blocks into `levels`, mirroring [`encode_chroma_tokens`].
fn decode_chroma_tokens(
    dec: &mut BoolDecoder,
    above: &mut EntropyCtx,
    left: &mut EntropyCtx,
    probs: &CoeffProbs,
    levels: &mut MbLevels,
) {
    for (plane_levels, above_ctx, left_ctx) in [
        (&mut levels.u, &mut above.u, &mut left.u),
        (&mut levels.v, &mut above.v, &mut left.v),
    ] {
        for i in 0..4 {
            let (r, c) = (i / 2, i % 2);
            let ctx = usize::from(above_ctx[c]) + usize::from(left_ctx[r]);
            let has = tokens::decode_block(dec, &mut plane_levels[i], 2, ctx, probs);
            above_ctx[c] = has;
            left_ctx[r] = has;
        }
    }
}

/// Decodes a whole-block (non-`B_PRED`) macroblock's coefficient blocks: Y2, 16 luma (plane 0), then
/// chroma.
fn decode_mb_tokens(
    dec: &mut BoolDecoder,
    above: &mut EntropyCtx,
    left: &mut EntropyCtx,
    probs: &CoeffProbs,
) -> MbLevels {
    let mut levels = MbLevels::default();
    let ctx = usize::from(above.y2) + usize::from(left.y2);
    let has = tokens::decode_block(dec, &mut levels.y2, 1, ctx, probs);
    above.y2 = has;
    left.y2 = has;
    for i in 0..16 {
        let (r, c) = (i / 4, i % 4);
        let ctx = usize::from(above.y[c]) + usize::from(left.y[r]);
        let has = tokens::decode_block(dec, &mut levels.y[i], 0, ctx, probs);
        above.y[c] = has;
        left.y[r] = has;
    }
    decode_chroma_tokens(dec, above, left, probs, &mut levels);
    levels
}

/// Encodes a [`Yuv420`] image as a VP8 key-frame bitstream (the `VP8 ` chunk payload), returning the
/// bitstream and the encoder's reconstruction (the tier-2 oracle: it must equal any decoder's output).
/// Uses the normal loop filter.
#[must_use]
pub fn encode_frame(yuv: &Yuv420, quant_index: u8) -> (Vec<u8>, FrameBuffers) {
    encode_frame_filtered(yuv, quant_index, EncodeOptions::default())
}

/// Encodes a frame with explicit [`EncodeOptions`] — the loop-filter type and whether to emit
/// quantizer segments. [`encode_frame`] uses the defaults (normal filter, unsegmented). This lets the
/// differential oracle drive the alternative encoder paths.
#[must_use]
pub fn encode_frame_filtered(
    yuv: &Yuv420,
    quant_index: u8,
    opts: EncodeOptions,
) -> (Vec<u8>, FrameBuffers) {
    let mut header = frame_header(yuv.width(), yuv.height(), quant_index, opts.simple_filter);
    if opts.segmented {
        header.segmentation = Segmentation {
            enabled: true,
            update_map: true,
            abs_delta: false,
            quantizer: SEGMENT_QUANT_DELTAS,
            filter_strength: [0; 4],
            tree_probs: [128, 128, 128],
        };
    }
    header.token_partitions = opts.partitions.max(1);
    let n = header.token_partitions as usize;
    let seg_qf = segment_quant_factors(&header);
    let mut recon = FrameBuffers::new(yuv.width(), yuv.height());

    let (yw, yh) = (recon.y_stride(), recon.mb_rows * 16);
    let (cw, ch) = (recon.c_stride(), recon.mb_rows * 8);
    let src_y = pad_plane(yuv.y(), yuv.width() as usize, yuv.height() as usize, yw, yh);
    let vcw = Yuv420::chroma_width(yuv.width()) as usize;
    let vch = Yuv420::chroma_height(yuv.height()) as usize;
    let src_u = pad_plane(yuv.u(), vcw, vch, cw, ch);
    let src_v = pad_plane(yuv.v(), vcw, vch, cw, ch);

    let segment_map: Vec<usize> = (0..recon.mb_rows * recon.mb_cols)
        .map(|i| {
            if header.segmentation.enabled {
                let (mbx, mby) = (i % recon.mb_cols, i / recon.mb_cols);
                (mb_luma_mean(&src_y, yw, mbx, mby) / 64).min(3) as usize
            } else {
                0
            }
        })
        .collect();

    let mut modes = BoolEncoder::new();
    header::write_frame_header(&mut modes, &header);
    let mut residuals: Vec<BoolEncoder> = (0..n).map(|_| BoolEncoder::new()).collect();
    let probs = &tokens::DEFAULT_COEFF_PROBS;

    let mut above = vec![EntropyCtx::default(); recon.mb_cols];
    let mut above_bmodes = vec![[B_DC_PRED; 4]; recon.mb_cols];
    let mut filter_interior = vec![false; recon.mb_cols * recon.mb_rows];
    for mb_y in 0..recon.mb_rows {
        let mut left = EntropyCtx::default();
        let mut left_bmodes = [B_DC_PRED; 4];
        for mb_x in 0..recon.mb_cols {
            let segment = segment_map[mb_y * recon.mb_cols + mb_x];
            let qf = seg_qf[segment];
            let uv_mode = select_chroma_mode(&recon, &src_u, &src_v, cw, mb_x, mb_y);
            let u_pred = predict_chroma(&recon.u, recon.c_stride(), mb_x, mb_y, uv_mode);
            let v_pred = predict_chroma(&recon.v, recon.c_stride(), mb_x, mb_y, uv_mode);

            // Whole-block luma candidate and its prediction SAD.
            let wb_mode = select_luma_mode(&recon, &src_y, yw, mb_x, mb_y);
            let wb_sad = block_sad(
                &predict_luma(&recon, mb_x, mb_y, wb_mode),
                &src_y,
                yw,
                mb_x,
                mb_y,
                16,
            );

            // B_PRED candidate — scribbles its reconstruction into recon.y while selecting submodes.
            let above_right = above_right_source(&recon, mb_x, mb_y);
            let (sub_modes, bpred_levels, bpred_sad) =
                encode_bpred_luma(&mut recon, &src_y, yw, mb_x, mb_y, &qf, &above_right);
            let use_bpred = bpred_sad + BPRED_SAD_PENALTY < wb_sad;

            let mut levels = MbLevels {
                u: quantize_chroma(&src_u, cw, mb_x, mb_y, &u_pred, &qf),
                v: quantize_chroma(&src_v, cw, mb_x, mb_y, &v_pred, &qf),
                ..Default::default()
            };
            // Compute the luma levels before writing modes so the skip flag — which precedes the luma
            // mode — reflects the whole macroblock. Whole-block luma is reconstructed afterward (B_PRED
            // was already reconstructed during submode selection).
            let wb_pred = (!use_bpred).then(|| predict_luma(&recon, mb_x, mb_y, wb_mode));
            if let Some(yp) = &wb_pred {
                quantize_luma(&src_y, yw, mb_x, mb_y, yp, &qf, &mut levels);
            } else {
                levels.y = bpred_levels;
            }
            let skip = !mb_has_coeffs(&levels);
            let y_mode = if use_bpred { B_PRED } else { wb_mode };

            if header.segmentation.update_map {
                modes.put_tree(MB_SEGMENT_TREE, &header.segmentation.tree_probs, segment);
            }
            modes.put_bool(header.prob_skip_false, skip);
            modes.put_tree(
                prediction::KF_YMODE_TREE,
                &prediction::KF_YMODE_PROB,
                y_mode,
            );
            if use_bpred {
                write_bmodes(&mut modes, &sub_modes, &above_bmodes[mb_x], &left_bmodes);
            }
            modes.put_tree(
                prediction::KF_UV_MODE_TREE,
                &prediction::KF_UV_MODE_PROB,
                uv_mode,
            );

            if let Some(yp) = &wb_pred {
                reconstruct_luma(&mut recon, mb_x, mb_y, yp, &levels, &qf);
            }
            let cstride = recon.c_stride();
            reconstruct_chroma(&mut recon.u, cstride, mb_x, mb_y, &u_pred, &levels.u, &qf);
            reconstruct_chroma(&mut recon.v, cstride, mb_x, mb_y, &v_pred, &levels.v, &qf);

            filter_interior[mb_y * recon.mb_cols + mb_x] = use_bpred || mb_has_coeffs(&levels);
            if skip {
                clear_mb_context(&mut above[mb_x], &mut left, use_bpred);
            } else {
                encode_mb_tokens(
                    &mut residuals[mb_y % n],
                    &mut above[mb_x],
                    &mut left,
                    probs,
                    &levels,
                    use_bpred,
                );
            }

            (above_bmodes[mb_x], left_bmodes) = bmode_propagation(use_bpred, wb_mode, &sub_modes);
        }
    }

    apply_loop_filter(&mut recon, &header.loop_filter, &filter_interior);

    let part0 = modes.finish();
    let token_parts: Vec<Vec<u8>> = residuals.into_iter().map(BoolEncoder::finish).collect();
    let mut out = Vec::new();
    header::write_uncompressed_chunk(&header, part0.len() as u32, &mut out);
    out.extend_from_slice(&part0);
    // The first N-1 token-partition sizes are stored as 3-byte little-endian prefixes (§9.5); the
    // last partition's size is implied by the remainder.
    for part in &token_parts[..n - 1] {
        let len = part.len() as u32;
        out.extend_from_slice(&[len as u8, (len >> 8) as u8, (len >> 16) as u8]);
    }
    for part in &token_parts {
        out.extend_from_slice(part);
    }
    (out, recon)
}

/// Splits the token-partition section (everything after the control partition) into `n` boolean
/// decoders (RFC 6386 §9.5): the first `n-1` partition sizes are 3-byte little-endian prefixes, the
/// last partition's size is the remainder.
fn split_token_partitions(data: &[u8], n: usize) -> Result<Vec<BoolDecoder<'_>>> {
    let sizes_len = (n - 1) * 3;
    if data.len() < sizes_len {
        return Err(Error::InvalidInput("VP8: token-partition sizes truncated"));
    }
    let mut decoders = Vec::with_capacity(n);
    let mut offset = sizes_len;
    for i in 0..n {
        let size = if i < n - 1 {
            let s = &data[i * 3..i * 3 + 3];
            usize::from(s[0]) | (usize::from(s[1]) << 8) | (usize::from(s[2]) << 16)
        } else {
            data.len() - offset
        };
        let end = offset
            .checked_add(size)
            .filter(|&e| e <= data.len())
            .ok_or(Error::InvalidInput("VP8: token partition exceeds frame"))?;
        decoders.push(BoolDecoder::new(&data[offset..end]));
        offset = end;
    }
    Ok(decoders)
}

/// Decodes a VP8 key-frame bitstream (the `VP8 ` chunk payload) into reconstructed planes.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] for a malformed stream or [`Error::Unsupported`] for features not
/// yet implemented (per-macroblock loop-filter adjustments, …).
pub fn decode_frame(data: &[u8]) -> Result<FrameBuffers> {
    let chunk = header::read_uncompressed_chunk(data)?;
    let part0_end = UNCOMPRESSED_CHUNK_LEN + chunk.first_partition_size as usize;
    if part0_end > data.len() {
        return Err(Error::InvalidInput("VP8: first partition exceeds frame"));
    }
    let mut modes = BoolDecoder::new(&data[UNCOMPRESSED_CHUNK_LEN..part0_end]);
    let (head, coeff_probs) = header::read_frame_header(&chunk, &mut modes)?;
    let seg_qf = segment_quant_factors(&head);
    let n = head.token_partitions as usize;
    let mut residuals = split_token_partitions(&data[part0_end..], n)?;
    let mut recon = FrameBuffers::new(u32::from(chunk.width), u32::from(chunk.height));

    let mut above = vec![EntropyCtx::default(); recon.mb_cols];
    let mut above_bmodes = vec![[B_DC_PRED; 4]; recon.mb_cols];
    let mut filter_interior = vec![false; recon.mb_cols * recon.mb_rows];
    for mb_y in 0..recon.mb_rows {
        let mut left = EntropyCtx::default();
        let mut left_bmodes = [B_DC_PRED; 4];
        for mb_x in 0..recon.mb_cols {
            let segment = if head.segmentation.update_map {
                modes.get_tree(MB_SEGMENT_TREE, &head.segmentation.tree_probs)
            } else {
                0
            };
            let qf = seg_qf[segment];
            let skip = head.mb_no_skip_coeff && modes.get_bool(head.prob_skip_false);
            let y_mode = modes.get_tree(prediction::KF_YMODE_TREE, &prediction::KF_YMODE_PROB);
            let is_bpred = y_mode == B_PRED;
            let sub_modes = if is_bpred {
                read_bmodes(&mut modes, &above_bmodes[mb_x], &left_bmodes)
            } else {
                [B_DC_PRED; 16]
            };
            let uv_mode = modes.get_tree(prediction::KF_UV_MODE_TREE, &prediction::KF_UV_MODE_PROB);
            let u_pred = predict_chroma(&recon.u, recon.c_stride(), mb_x, mb_y, uv_mode);
            let v_pred = predict_chroma(&recon.v, recon.c_stride(), mb_x, mb_y, uv_mode);
            let cstride = recon.c_stride();

            // A skipped macroblock has no coefficients: its residual is zero (the reconstruction is the
            // prediction) and no tokens are read.
            let mut levels = MbLevels::default();
            if is_bpred {
                let above_right = above_right_source(&recon, mb_x, mb_y);
                if skip {
                    reconstruct_bpred_zero(&mut recon, mb_x, mb_y, &sub_modes, &above_right);
                } else {
                    decode_bpred_luma(
                        &mut recon,
                        &mut residuals[mb_y % n],
                        &mut above[mb_x],
                        &mut left,
                        &coeff_probs,
                        mb_x,
                        mb_y,
                        &qf,
                        &sub_modes,
                        &above_right,
                    );
                    decode_chroma_tokens(
                        &mut residuals[mb_y % n],
                        &mut above[mb_x],
                        &mut left,
                        &coeff_probs,
                        &mut levels,
                    );
                }
                reconstruct_chroma(&mut recon.u, cstride, mb_x, mb_y, &u_pred, &levels.u, &qf);
                reconstruct_chroma(&mut recon.v, cstride, mb_x, mb_y, &v_pred, &levels.v, &qf);
                filter_interior[mb_y * recon.mb_cols + mb_x] = true; // B_PRED always filters interiors
            } else {
                let y_pred = predict_luma(&recon, mb_x, mb_y, y_mode);
                if !skip {
                    levels = decode_mb_tokens(
                        &mut residuals[mb_y % n],
                        &mut above[mb_x],
                        &mut left,
                        &coeff_probs,
                    );
                }
                reconstruct_luma(&mut recon, mb_x, mb_y, &y_pred, &levels, &qf);
                reconstruct_chroma(&mut recon.u, cstride, mb_x, mb_y, &u_pred, &levels.u, &qf);
                reconstruct_chroma(&mut recon.v, cstride, mb_x, mb_y, &v_pred, &levels.v, &qf);
                filter_interior[mb_y * recon.mb_cols + mb_x] = mb_has_coeffs(&levels);
            }
            if skip {
                clear_mb_context(&mut above[mb_x], &mut left, is_bpred);
            }

            (above_bmodes[mb_x], left_bmodes) = bmode_propagation(is_bpred, y_mode, &sub_modes);
        }
    }

    apply_loop_filter(&mut recon, &head.loop_filter, &filter_interior);
    Ok(recon)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a `Yuv420` from a deterministic synthetic pattern.
    fn pattern(width: u32, height: u32) -> Yuv420 {
        let (w, h) = (width as usize, height as usize);
        let (cw, ch) = (
            Yuv420::chroma_width(width) as usize,
            Yuv420::chroma_height(height) as usize,
        );
        let y = (0..w * h)
            .map(|i| ((i * 7 + i / w * 13) & 0xff) as u8)
            .collect();
        let u = (0..cw * ch).map(|i| ((i * 5 + 64) & 0xff) as u8).collect();
        let v = (0..cw * ch)
            .map(|i| ((i * 11 + 128) & 0xff) as u8)
            .collect();
        Yuv420::new(width, height, y, u, v).unwrap()
    }

    /// Builds B_PRED-favorable content: each 4×4 region carries a different gradient direction, so a
    /// single whole-block mode predicts the macroblock poorly but per-subblock modes do not.
    fn detailed(width: u32, height: u32) -> Yuv420 {
        let (w, h) = (width as usize, height as usize);
        let (cw, ch) = (
            Yuv420::chroma_width(width) as usize,
            Yuv420::chroma_height(height) as usize,
        );
        let y = (0..w * h)
            .map(|i| {
                let (x, yy) = (i % w, i / w);
                let v = match (x / 4 + yy / 4) % 4 {
                    0 => x * 18,
                    1 => yy * 18,
                    2 => (x + yy) * 18,
                    _ => x.wrapping_sub(yy).wrapping_mul(18),
                };
                (v & 0xff) as u8
            })
            .collect();
        let u = (0..cw * ch).map(|i| ((i * 3) & 0xff) as u8).collect();
        let v = (0..cw * ch).map(|i| ((i * 9 + 70) & 0xff) as u8).collect();
        Yuv420::new(width, height, y, u, v).unwrap()
    }

    /// Counts macroblocks coded as `B_PRED` by re-reading partition 0, to confirm the path is
    /// genuinely exercised (not merely available).
    /// Re-reads partition 0 (modes) and returns `(B_PRED macroblocks, skipped macroblocks)`, to
    /// confirm those paths are genuinely exercised.
    fn mode_stats(data: &[u8]) -> (usize, usize) {
        let chunk = header::read_uncompressed_chunk(data).unwrap();
        let part0_end = UNCOMPRESSED_CHUNK_LEN + chunk.first_partition_size as usize;
        let mut modes = BoolDecoder::new(&data[UNCOMPRESSED_CHUNK_LEN..part0_end]);
        let (head, _) = header::read_frame_header(&chunk, &mut modes).unwrap();
        let mb_cols = (chunk.width as usize).div_ceil(16);
        let mb_rows = (chunk.height as usize).div_ceil(16);
        let mut above_bmodes = vec![[B_DC_PRED; 4]; mb_cols];
        let (mut bpred, mut skipped) = (0, 0);
        for _ in 0..mb_rows {
            let mut left_bmodes = [B_DC_PRED; 4];
            for mb_x in 0..mb_cols {
                if head.segmentation.update_map {
                    let _ = modes.get_tree(MB_SEGMENT_TREE, &head.segmentation.tree_probs);
                }
                if head.mb_no_skip_coeff && modes.get_bool(head.prob_skip_false) {
                    skipped += 1;
                }
                let y_mode = modes.get_tree(prediction::KF_YMODE_TREE, &prediction::KF_YMODE_PROB);
                let is_bpred = y_mode == B_PRED;
                let sub_modes = if is_bpred {
                    bpred += 1;
                    read_bmodes(&mut modes, &above_bmodes[mb_x], &left_bmodes)
                } else {
                    [B_DC_PRED; 16]
                };
                let _ = modes.get_tree(prediction::KF_UV_MODE_TREE, &prediction::KF_UV_MODE_PROB);
                (above_bmodes[mb_x], left_bmodes) = bmode_propagation(is_bpred, y_mode, &sub_modes);
            }
        }
        (bpred, skipped)
    }

    #[test]
    fn bpred_is_exercised_and_bit_exact() {
        let yuv = detailed(48, 48);
        let (bitstream, recon) = encode_frame(&yuv, 8);
        assert!(
            mode_stats(&bitstream).0 > 0,
            "detailed content should select B_PRED for some macroblocks"
        );
        let decoded = decode_frame(&bitstream).expect("decode");
        let (enc, dec) = (recon.to_yuv420(), decoded.to_yuv420());
        assert_eq!(enc.y(), dec.y(), "B_PRED luma mismatch");
        assert_eq!(enc.u(), dec.u(), "B_PRED u mismatch");
        assert_eq!(enc.v(), dec.v(), "B_PRED v mismatch");
    }

    #[test]
    fn mb_skip_is_exercised_and_bit_exact() {
        // A flat image predicts to 128 with a zero residual, so every macroblock is skipped; the
        // decode must reproduce it from the skip flags alone.
        let (w, h) = (48u32, 48u32);
        let (cw, ch) = (
            Yuv420::chroma_width(w) as usize,
            Yuv420::chroma_height(h) as usize,
        );
        let yuv = Yuv420::new(
            w,
            h,
            vec![128u8; (w * h) as usize],
            vec![128u8; cw * ch],
            vec![128u8; cw * ch],
        )
        .unwrap();
        let (bits, recon) = encode_frame(&yuv, 60);
        assert!(
            mode_stats(&bits).1 > 0,
            "flat content should skip macroblocks"
        );
        let dec = decode_frame(&bits).expect("decode");
        assert_eq!(recon.to_yuv420().y(), dec.to_yuv420().y());
        assert_eq!(recon.to_yuv420().u(), dec.to_yuv420().u());
        assert_eq!(recon.to_yuv420().v(), dec.to_yuv420().v());
    }

    /// Tier-2: the encoder's reconstruction must equal the native decoder's output, bit-for-bit.
    fn assert_encoder_recon_matches_decoder(width: u32, height: u32, q: u8) {
        let yuv = pattern(width, height);
        let (bitstream, recon) = encode_frame(&yuv, q);
        let decoded = decode_frame(&bitstream).expect("decode");
        let enc = recon.to_yuv420();
        let dec = decoded.to_yuv420();
        assert_eq!(enc.y(), dec.y(), "luma mismatch at {width}x{height} q{q}");
        assert_eq!(enc.u(), dec.u(), "u mismatch at {width}x{height} q{q}");
        assert_eq!(enc.v(), dec.v(), "v mismatch at {width}x{height} q{q}");
    }

    #[test]
    fn encoder_recon_matches_decoder_across_sizes_and_quant() {
        for &(w, h) in &[
            (16u32, 16u32),
            (32, 16),
            (17, 9),
            (1, 1),
            (64, 48),
            (33, 41),
        ] {
            for &q in &[0u8, 10, 40, 80, 127] {
                assert_encoder_recon_matches_decoder(w, h, q);
            }
        }
    }

    #[test]
    fn both_loop_filters_reconstruct_bit_exact() {
        // The simple (luma-only) and normal (luma+chroma) filters must each reconstruct identically
        // in the encoder and decoder — exercising both decoder filter paths on coefficient-bearing
        // content (so interior edges are filtered too).
        for simple in [true, false] {
            for &q in &[20u8, 60, 110] {
                let yuv = detailed(48, 32);
                let opts = EncodeOptions {
                    simple_filter: simple,
                    segmented: false,
                    partitions: 1,
                };
                let (bits, recon) = encode_frame_filtered(&yuv, q, opts);
                let dec = decode_frame(&bits).expect("decode");
                let (enc, dec) = (recon.to_yuv420(), dec.to_yuv420());
                assert_eq!(enc.y(), dec.y(), "luma simple={simple} q{q}");
                assert_eq!(enc.u(), dec.u(), "u simple={simple} q{q}");
                assert_eq!(enc.v(), dec.v(), "v simple={simple} q{q}");
            }
        }
    }

    #[test]
    fn segmentation_round_trips_bit_exact() {
        // Four quantizer segments (assigned by macroblock luma mean) must reconstruct identically in
        // the encoder and the decoder across a range of base quantizers.
        for &q in &[10u8, 40, 90] {
            let yuv = detailed(64, 48);
            let opts = EncodeOptions {
                simple_filter: false,
                segmented: true,
                partitions: 1,
            };
            let (bits, recon) = encode_frame_filtered(&yuv, q, opts);
            let dec = decode_frame(&bits).expect("decode");
            let (enc, dec) = (recon.to_yuv420(), dec.to_yuv420());
            assert_eq!(enc.y(), dec.y(), "luma q{q}");
            assert_eq!(enc.u(), dec.u(), "u q{q}");
            assert_eq!(enc.v(), dec.v(), "v q{q}");
        }
    }

    #[test]
    fn token_partitions_round_trip_bit_exact() {
        // 1/2/4/8 token partitions must each reconstruct identically; a tall image routes macroblock
        // rows across all eight partitions.
        for partitions in [1u8, 2, 4, 8] {
            let yuv = detailed(32, 160);
            let opts = EncodeOptions {
                simple_filter: false,
                segmented: false,
                partitions,
            };
            let (bits, recon) = encode_frame_filtered(&yuv, 30, opts);
            let dec = decode_frame(&bits).expect("decode");
            let (enc, dec) = (recon.to_yuv420(), dec.to_yuv420());
            assert_eq!(enc.y(), dec.y(), "luma p{partitions}");
            assert_eq!(enc.u(), dec.u(), "u p{partitions}");
            assert_eq!(enc.v(), dec.v(), "v p{partitions}");
        }
    }

    #[test]
    fn decode_rejects_truncated_first_partition() {
        let yuv = pattern(16, 16);
        let (mut bitstream, _) = encode_frame(&yuv, 40);
        bitstream.truncate(UNCOMPRESSED_CHUNK_LEN + 1);
        let _ = decode_frame(&bitstream);
    }
}
