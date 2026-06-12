//! The all-intra tile encoder: superblock/partition iteration (§5.11.2/.4), intra prediction
//! (§7.11.2), the forward transform (via `gamut-dsp`), and coefficient coding with full context
//! derivation (§5.11.39, §8.3.2). One or more uniform tiles (§5.11.1) are coded with this path.
//!
//! Two paths share this code, selected by `qindex` (`base_q_idx`):
//! - **Lossless** (`qindex == 0`): forced `TX_4X4` Walsh–Hadamard; prediction neighbours are the
//!   source samples (which equal the reconstruction under lossless); partitions are
//!   `PARTITION_NONE` except at the right/bottom frame edges.
//! - **Lossy** (`qindex > 0`): recursive `PARTITION_NONE`/`HORZ`/`VERT`/`SPLIT` down to 4×4, with
//!   per-superblock `delta_q`/`delta_lf` and optional segmentation (`SEG_LVL_ALT_Q`). Each luma
//!   block codes a transform under `TX_MODE_SELECT` (square `tx_depth` 0..2; sizes `TX_4X4`..
//!   `TX_64X64`, with `TX_32X32`/`TX_64X64` DCT-only). The luma prediction mode is chosen per block
//!   from all 13 intra modes (`DC`, the eight directional `V`/`H`/`D45`/`D135`/`D113`/`D157`/`D203`/
//!   `D67` with `angle_delta`, and `SMOOTH`/`SMOOTH_V`/`SMOOTH_H`/`PAETH`, §7.11.2), plus recursive
//!   **filter-intra** (§7.11.2.3, signaled as `DC_PRED` + `use_filter_intra`) and **palette**
//!   (§7.11.4); the luma transform type is chosen from `TX_SET_INTRA_2`
//!   (`{IDTX, DCT_DCT, ADST_ADST, ADST_DCT, DCT_ADST}`) and signaled. Chroma is `DC_PRED` or
//!   **chroma-from-luma** (`UV_CFL_PRED`, §7.11.5, when it beats DC) + `DCT_DCT`. Prediction reads a
//!   **reconstruction buffer** that the encoder maintains exactly as the decoder would (predict →
//!   transform → quantize → dequantize → inverse-transform → add → store), so the encoder's
//!   reconstruction is bit-exact with a conformant decoder's output; the in-loop filters
//!   (deblocking, CDEF, loop restoration, superres) are applied afterward in `filter`.
//!
//! The frame is coded on the MI-unit grid (`mi_cols*4 × mi_rows*4`, dimensions rounded up to a
//! multiple of 8); the out-of-frame padding is edge-replicated and cropped away on decode.

use crate::cdf;
use crate::quant::{ac_q, dc_q, dequant, quantize};
use crate::transform::{TxSize, TxType, forward_transform_2d, inverse_transform_2d};
use gamut_bitstream::SymbolEncoder;
use gamut_color::{Planar8, clip_pixel};
use gamut_dsp::round2_signed;

/// `NUM_BASE_LEVELS` (§3).
const NUM_BASE_LEVELS: i32 = 2;
/// `NUM_BASE_LEVELS + COEFF_BASE_RANGE`, the golomb threshold (§5.11.39).
const COEFF_BASE_PLUS_RANGE: i32 = 14;

/// `DC_PRED` (§3): the only mode used for chroma and the lossless path.
const DC_PRED: u8 = 0;
/// `V_PRED` (§3): directional, `pAngle = 90` (copy the above row).
const V_PRED: u8 = 1;
/// `H_PRED` (§3): directional, `pAngle = 180` (copy the left column).
const H_PRED: u8 = 2;
/// `D45_PRED` (§3): directional, `pAngle = 45` (zone 1, needs above-right samples).
const D45_PRED: u8 = 3;
/// `D203_PRED` (§3): directional, `pAngle = 203` (zone 3, needs below-left samples).
const D203_PRED: u8 = 7;
/// `D67_PRED` (§3): directional, `pAngle = 67` (zone 1, needs above-right samples).
const D67_PRED: u8 = 8;
/// `D135_PRED` (§3): directional, `pAngle = 135`.
const D135_PRED: u8 = 4;
/// `D113_PRED` (§3): directional, `pAngle = 113`.
const D113_PRED: u8 = 5;
/// `D157_PRED` (§3): directional, `pAngle = 157`.
const D157_PRED: u8 = 6;
/// `SMOOTH_PRED` (§3).
const SMOOTH_PRED: u8 = 9;
/// `SMOOTH_V_PRED` (§3).
const SMOOTH_V_PRED: u8 = 10;
/// `SMOOTH_H_PRED` (§3).
const SMOOTH_H_PRED: u8 = 11;
/// `PAETH_PRED` (§3).
const PAETH_PRED: u8 = 12;
/// `UV_CFL_PRED` (§3): the chroma-from-luma `uv_mode` value (the 14th symbol of the CfL-allowed
/// `uv_mode` CDF). Selected for chroma on the lossy path when CfL beats plain `DC_PRED`.
const UV_CFL_PRED: u8 = 13;
/// The luma intra modes the lossy path searches over. All are bit-exact from only the immediate
/// above/left neighbours plus the top-left corner: the non-directional modes by construction, and
/// the included directional modes because `angle_delta = 0` (4×4 blocks) and `pAngle ∈ {90, 180}`
/// (cardinal) or `90 < pAngle < 180` (zone 2) keep every reference index within `[-1, 3]`. The
/// directional angles `D45`/`D67` (zone 1) and `D203` (zone 3) additionally read the above-right /
/// below-left reconstruction via the `BlockDecoded` availability map.
const LUMA_MODES: [u8; 13] = [
    DC_PRED,
    V_PRED,
    H_PRED,
    D45_PRED,
    D135_PRED,
    D113_PRED,
    D157_PRED,
    D203_PRED,
    D67_PRED,
    SMOOTH_PRED,
    SMOOTH_V_PRED,
    SMOOTH_H_PRED,
    PAETH_PRED,
];

/// The eight directional luma modes, in `YMode` order (`V`..`D67`), used to drive the ≥8×8 angle
/// search. Each is a candidate base angle that `AngleDeltaY ∈ [-3, 3]` fine-tunes.
const DIRECTIONAL_MODES: [u8; 8] = [
    V_PRED, H_PRED, D45_PRED, D135_PRED, D113_PRED, D157_PRED, D203_PRED, D67_PRED,
];

/// `Filter_Intra_Mode_To_Intra_Dir[INTRA_FILTER_MODES]` (§9.3): maps a filter-intra mode to the
/// `intraDir` used to select the 16×16 transform-type CDF (§9.4). Order:
/// `{FILTER_DC, FILTER_V, FILTER_H, FILTER_D157, FILTER_PAETH}`.
const FILTER_INTRA_MODE_TO_INTRA_DIR: [u8; 5] = [DC_PRED, V_PRED, H_PRED, D157_PRED, DC_PRED];

/// True for the eight directional intra modes (`V_PRED..=D67_PRED`, mode indices `1..=8`), which is
/// `is_directional_mode` (§7.11.2). These are the modes for which `angle_delta_y` is signaled (≥8×8)
/// and whose prediction follows the directional process (§7.11.2.4).
const fn is_directional(mode: u8) -> bool {
    V_PRED <= mode && mode <= D67_PRED
}

/// A selected luma palette for a block (§5.11.46/.49): the sorted distinct colors and the per-pixel
/// `ColorMapY` (index into `colors`, row-major over the block).
struct PaletteBlock {
    colors: Vec<u8>,
    index_map: Vec<u8>,
}

/// The `palette_color_idx_y` CDF for a luma palette of size `n` (2..=8) at color context `ctx`
/// (§9.4). `n` is the palette size; `ctx` is `Palette_Color_Context[hash]`.
fn palette_color_cdf(n: usize, ctx: usize) -> &'static [u16] {
    match n {
        2 => &cdf::PALETTE_SIZE_2_Y_COLOR[ctx],
        3 => &cdf::PALETTE_SIZE_3_Y_COLOR[ctx],
        4 => &cdf::PALETTE_SIZE_4_Y_COLOR[ctx],
        5 => &cdf::PALETTE_SIZE_5_Y_COLOR[ctx],
        6 => &cdf::PALETTE_SIZE_6_Y_COLOR[ctx],
        7 => &cdf::PALETTE_SIZE_7_Y_COLOR[ctx],
        _ => &cdf::PALETTE_SIZE_8_Y_COLOR[ctx],
    }
}

/// `CeilLog2(x)` (§4.7): the smallest `k` with `2^k >= x` (0 for `x <= 1`).
fn ceil_log2(x: usize) -> u32 {
    if x < 2 { 0 } else { (x - 1).ilog2() + 1 }
}

/// `get_palette_color_context` (§5.11.50): from the three already-decoded neighbours of `(r, c)` in
/// the `bw`-wide `color_map`, computes the reordered `ColorOrder` (most-likely color first) and the
/// color context. `n` is the palette size.
fn palette_color_context(
    color_map: &[u8],
    bw: usize,
    r: usize,
    c: usize,
    n: usize,
) -> ([usize; 8], usize) {
    let mut scores = [0i32; 8];
    let mut order = [0usize; 8];
    for (i, o) in order.iter_mut().enumerate() {
        *o = i;
    }
    if c > 0 {
        scores[color_map[r * bw + (c - 1)] as usize] += 2;
    }
    if r > 0 && c > 0 {
        scores[color_map[(r - 1) * bw + (c - 1)] as usize] += 1;
    }
    if r > 0 {
        scores[color_map[(r - 1) * bw + c] as usize] += 2;
    }
    // Partial selection sort of the first PALETTE_NUM_NEIGHBORS = 3 by descending score, rotating the
    // chosen entry into place.
    for i in 0..3 {
        let mut max_idx = i;
        for j in (i + 1)..n {
            if scores[j] > scores[max_idx] {
                max_idx = j;
            }
        }
        if max_idx != i {
            let (ms, mo) = (scores[max_idx], order[max_idx]);
            let mut k = max_idx;
            while k > i {
                scores[k] = scores[k - 1];
                order[k] = order[k - 1];
                k -= 1;
            }
            scores[i] = ms;
            order[i] = mo;
        }
    }
    let hash = (scores[0] + scores[1] * 2 + scores[2] * 2) as usize;
    (order, cdf::PALETTE_COLOR_CONTEXT[hash] as usize)
}

/// Per-segment `SEG_LVL_ALT_Q` quantizer deltas (§5.9.14): `Some(delta)` enables the feature on that
/// segment (its block quantizer is `Clip3(0, 255, CurrentQIndex + delta)`), `None` leaves the segment
/// on the frame quantizer. The header signals this exact table, so encoder and decoder agree. The two
/// active segments give the spatial `segment_id` prediction real work without `SegIdPreSkip`
/// (`SEG_LVL_ALT_Q` < `SEG_LVL_REF_FRAME`). `LastActiveSegId` is the highest `Some` index (2 here).
///
/// The deltas are **positive** so a segment's quantizer is always `> 0`: a segment whose
/// `get_qindex` is 0 would be *coded-lossless* (§7.12.2) and require the Walsh–Hadamard path, which
/// this lossy encoder does not select per-segment.
pub(crate) const SEG_ALT_Q: [Option<i32>; 8] =
    [None, Some(20), Some(50), None, None, None, None, None];

/// `LastActiveSegId` (§5.9.14): the highest segment index with an enabled feature.
const LAST_ACTIVE_SEG: i32 = 2;

/// `neg_interleave(x, ref, max)` — the encoder inverse of the decoder's `neg_deinterleave` (§5.11.9):
/// maps the segment id `x` to the difference coded relative to the predicted id `ref`.
fn neg_interleave(x: i32, r: i32, max: i32) -> i32 {
    if r == 0 {
        return x;
    }
    if r >= max - 1 {
        return max - x - 1;
    }
    let bounded = if 2 * r < max {
        (x - r).abs() <= r
    } else {
        (x - r).abs() < max - r
    };
    if bounded {
        if x <= r { (r - x) * 2 } else { (x - r) * 2 - 1 }
    } else if 2 * r < max {
        x
    } else {
        max - (x + 1)
    }
}

/// The coefficient-CDF quantizer context (§8.3.2) for a quantizer index: `0` if `≤ 20`, `1` if
/// `≤ 60`, `2` if `≤ 120`, else `3`.
const fn qctx_for(qindex: i32) -> usize {
    if qindex <= 20 {
        0
    } else if qindex <= 60 {
        1
    } else if qindex <= 120 {
        2
    } else {
        3
    }
}

/// The square transform size of the given side length (4/8/16/32). Used to map a `bw >> tx_depth`
/// width back to a [`TxSize`] under `TX_MODE_SELECT`.
const fn square_tx(width: usize) -> TxSize {
    match width {
        4 => TxSize::Tx4x4,
        8 => TxSize::Tx8x8,
        16 => TxSize::Tx16x16,
        32 => TxSize::Tx32x32,
        _ => TxSize::Tx64x64,
    }
}

/// Whether a square block of `num4x4 × num4x4` MI cells at MI position `(r, c)` overhangs the
/// `mi_rows × mi_cols` frame, i.e. does not fully fit. AV1 still codes such a block (a
/// `PARTITION_NONE` is allowed when only the top/left half is in-frame), but the lossy
/// reconstruction buffer is the MI-frame size, so an overhanging block must instead be force-split
/// (see [`FrameEncoder::encode_partition`]). A block exactly reaching an edge (`r + num4x4 ==
/// mi_rows`) fits and is not flagged.
const fn block_exceeds_frame(
    r: usize,
    c: usize,
    num4x4: usize,
    mi_rows: usize,
    mi_cols: usize,
) -> bool {
    r + num4x4 > mi_rows || c + num4x4 > mi_cols
}

/// The transform size that exactly covers a `w × h` block (the rectangular block's `Max_Tx_Size`
/// under `TX_MODE_LARGEST`): a square block maps to its square transform, a rectangular one to the
/// matching rectangular transform.
const fn rect_tx(w: usize, h: usize) -> TxSize {
    match (w, h) {
        (16, 8) => TxSize::Tx16x8,
        (8, 16) => TxSize::Tx8x16,
        (32, 16) => TxSize::Tx32x16,
        (16, 32) => TxSize::Tx16x32,
        _ => square_tx(w),
    }
}

/// How a single transform block's samples are predicted. `mode` is the intra mode (`DC_PRED` for
/// chroma); `angle_delta` is the directional fine angle (`AngleDeltaY`, signaled only for ≥8×8 luma
/// directional blocks); `filter_intra` overrides luma prediction with a recursive filter-intra mode
/// (§7.11.2.3); `cfl_alpha` adds the chroma-from-luma high-frequency term (§7.11.5). At most one of
/// the latter two is set, and only on its respective plane.
#[derive(Clone, Copy)]
struct Pred {
    mode: u8,
    angle_delta: i8,
    filter_intra: Option<u8>,
    cfl_alpha: Option<i32>,
}

/// The reconstructed (decoded) planes on the coded MI grid, used as prediction neighbours in the
/// lossy path and exported for the bit-exact decoder cross-check.
pub(crate) struct Reconstruction {
    pub planes: [Vec<u16>; 3],
    pub coded_w: usize,
    /// Bits per sample (8, 10, or 12); the planes carry values in `0..=(1 << bit_depth) - 1`.
    pub bit_depth: u32,
    /// Post-deblock (pre-CDEF) luma, retained for the loop-restoration stripe boundaries. Loop
    /// restoration is applied by the caller **after** any superres upscale (§7.4 order:
    /// deblock → CDEF → superres → loop restoration), so both the CDEF luma (`planes[0]`) and this
    /// are upscaled first when superres is active. Empty on the lossless path.
    pub deblocked_luma: Vec<u16>,
}

/// Encoder for the single tile that spans the whole frame.
pub(crate) struct FrameEncoder<'a> {
    planes: [&'a [u8]; 3],
    width: usize,
    height: usize,
    mi_cols: usize,
    mi_rows: usize,
    coded_w: usize,
    coded_h: usize,
    /// MI boundaries of the tile currently being encoded (`0`/`mi_cols`/`mi_rows` for a single tile).
    /// A tile decodes independently, so neighbour availability (prediction samples and symbol
    /// contexts) stops at the tile edge even when it is not the frame edge.
    tile_c0: usize,
    tile_x0: usize,
    /// Exclusive right/bottom MI boundary of the current tile (`MiColEnd`/`MiRowEnd`).
    tile_c1: usize,
    tile_r1: usize,
    /// `base_q_idx`; 0 ⇒ lossless WHT path, > 0 ⇒ lossy DCT path.
    qindex: u8,
    /// Coefficient-CDF quantizer context (§8.3.2): 0 if `base_q_idx ≤ 20`, 1 if ≤ 60, 2 if ≤ 120,
    /// else 3. Frame-level — `init_coeff_cdfs` derives it from `base_q_idx`, so it is **not** changed
    /// by the per-superblock `delta_q` (only the `dc_quant`/`ac_quant` step sizes follow that).
    qctx: usize,
    dc_quant: i32,
    ac_quant: i32,
    /// `CurrentQIndex` (§7.12.2): `base_q_idx` plus the accumulated per-superblock `delta_q`.
    current_qindex: i32,
    /// `CurrentDeltaLF` (§7.14.4): the accumulated per-superblock loop-filter-level delta.
    current_dlf: i32,
    /// Whether `delta_q_present` (lossy path); when set, the first block of each superblock signals a
    /// `delta_q` and `read_deltas` gates it.
    delta_q_present: bool,
    read_deltas: bool,
    /// Bits per sample (8, 10, or 12); sets the reconstruction clamp range and dequant/transform
    /// bit depth.
    bit_depth: u32,
    /// Reconstructed samples per plane on the coded grid (lossy path only), each in
    /// `0..=(1 << bit_depth) - 1`.
    recon: [Vec<u16>; 3],
    sym: SymbolEncoder,
    above_level: [Vec<u8>; 3],
    above_dc: [Vec<u8>; 3],
    left_level: [Vec<u8>; 3],
    left_dc: [Vec<u8>; 3],
    /// `Mi_Width_Log2` of the block covering each MI cell (for the partition context).
    mi_bsl: Vec<u8>,
    /// `Mi_Height_Log2` of the block covering each MI cell (the height counterpart of `mi_bsl`), for
    /// the deblock filter to size a rectangular block's chroma horizontal edges.
    mi_bsl_h: Vec<u8>,
    /// `AbovePartitionContext`/`LeftPartitionContext` (§5.11.4): per-column / per-row partition
    /// context bitmasks updated from `AL_PART_CTX` after each terminal partition. A given block level
    /// reads bit `bsl - 1`; this distinguishes HORZ/VERT/NONE neighbours (which `mi_bsl` alone cannot).
    above_partition: Vec<u8>,
    left_partition: Vec<u8>,
    /// The luma intra mode (`YMode`) chosen for each MI cell, for the `intra_frame_y_mode` context.
    mi_ymode: Vec<u8>,
    /// The `skip` flag of the block covering each MI cell, for the `skip` context (§8.3.2).
    mi_skip: Vec<u8>,
    /// `SegmentIds` of the block covering each MI cell (§5.11.9), for the `segment_id` spatial
    /// prediction and per-segment quantizer.
    mi_segid: Vec<u8>,
    /// `PaletteSizeY` of the block covering each MI cell (0 = no palette), for the `has_palette_y`
    /// context and `get_palette_cache`.
    mi_psize: Vec<u8>,
    /// `PaletteColors` (the sorted luma palette, up to 8 entries) of the block covering each MI cell,
    /// for `get_palette_cache` (only the first `mi_psize` entries are meaningful).
    mi_pcolors: Vec<[u8; 8]>,
    /// `DeltaLF` (loop-filter-level delta) of the block covering each MI cell (§7.14.4), consumed by
    /// the deblocking filter to vary the level per superblock.
    mi_dlf: Vec<i8>,
    /// `Tx_Width_Log2` of the transform covering each MI cell (`2` for 4×4, `3` for 8×8), consumed by
    /// the deblocking loop filter to locate transform edges and pick the filter size.
    tx_log2: Vec<u8>,
    /// `Tx_Height_Log2` of the transform covering each MI cell — equals `tx_log2` for square
    /// transforms, differs for rectangular ones. The `tx_depth` context reads a left neighbour's tx
    /// *height* and an above neighbour's tx *width*, so both are tracked.
    tx_log2_h: Vec<u8>,
    /// Top-left MI position of the superblock currently being encoded (for `BlockDecoded` indexing).
    sb_r: usize,
    sb_c: usize,
    /// `BlockDecoded` (§5.11.3) for the current superblock's luma plane: one flag per 4×4, indexed
    /// `[(by + 1) * BD_STRIDE + (bx + 1)]` for superblock-relative `(by, bx)` in `-1..=16`. Drives
    /// the above-right / below-left reference-sample availability for directional prediction.
    block_decoded: Vec<u8>,
}

/// Row stride of [`FrameEncoder::block_decoded`]: a 64×64 superblock is 16 MI wide, plus the `-1`
/// guard row/column and the `+16` below/right edge ⇒ indices `-1..=16` → 18 entries.
const BD_STRIDE: usize = 18;

/// `Partition_Context` lookup (`dav1d_al_part_ctx[2][N_BL_LEVELS][PARTITION]`, §5.11.4), restricted to
/// the partition types this encoder emits: `[above|left][block level 0=128…4=8][NONE|HORZ|VERT|SPLIT]`.
/// After a terminal partition, the block's MI cells take these context bytes; a later block reads bit
/// `bsl - 1` of the neighbour byte to form its partition context. The SPLIT column is only ever read at
/// 8×8 (`bl == 4`) — at coarser levels a split recurses and the column (0) is unused.
const AL_PART_CTX: [[[u8; 4]; 5]; 2] = [
    [
        [0x00, 0x00, 0x10, 0x00],
        [0x10, 0x10, 0x18, 0x00],
        [0x18, 0x18, 0x1c, 0x00],
        [0x1c, 0x1c, 0x1e, 0x00],
        [0x1e, 0x1e, 0x1f, 0x1f],
    ],
    [
        [0x00, 0x10, 0x00, 0x00],
        [0x10, 0x18, 0x10, 0x00],
        [0x18, 0x1c, 0x18, 0x00],
        [0x1c, 0x1e, 0x1c, 0x00],
        [0x1e, 0x1f, 0x1e, 0x1f],
    ],
];

/// `Sm_Weights` (§9.3) for a transform dimension `size` (8/16/32/64). The SMOOTH predictors weight
/// each axis independently, so a rectangular block reads a different-length table per axis.
fn sm_weights(size: usize) -> &'static [i32] {
    match size {
        8 => &cdf::SM_WEIGHTS_8X8,
        16 => &cdf::SM_WEIGHTS_16X16,
        32 => &cdf::SM_WEIGHTS_32X32,
        _ => &cdf::SM_WEIGHTS_64X64,
    }
}

impl<'a> FrameEncoder<'a> {
    /// Creates an encoder over the 4:4:4 identity planes (Y=G, U=B, V=R) at quantizer `qindex`
    /// (`base_q_idx`; 0 selects the lossless path).
    pub(crate) fn new(planes: &'a Planar8, qindex: u8) -> Self {
        let width = planes.width() as usize;
        let height = planes.height() as usize;
        let mi_cols = 2 * ((width + 7) >> 3);
        let mi_rows = 2 * ((height + 7) >> 3);
        let coded_w = mi_cols * 4;
        let coded_h = mi_rows * 4;
        // 8-bit input today; the buffer and clamp range are bit-depth-generic for the M2 high-bit-
        // depth path.
        let bit_depth = 8u32;
        let recon = if qindex > 0 {
            [
                vec![0u16; coded_w * coded_h],
                vec![0u16; coded_w * coded_h],
                vec![0u16; coded_w * coded_h],
            ]
        } else {
            [Vec::new(), Vec::new(), Vec::new()]
        };
        Self {
            planes: [planes.plane(0), planes.plane(1), planes.plane(2)],
            width,
            height,
            mi_cols,
            mi_rows,
            coded_w,
            coded_h,
            tile_c0: 0,
            tile_x0: 0,
            tile_c1: mi_cols,
            tile_r1: mi_rows,
            qindex,
            qctx: qctx_for(i32::from(qindex)),
            dc_quant: dc_q(bit_depth, i32::from(qindex)),
            ac_quant: ac_q(bit_depth, i32::from(qindex)),
            current_qindex: i32::from(qindex),
            current_dlf: 0,
            delta_q_present: qindex > 0,
            read_deltas: false,
            bit_depth,
            recon,
            sym: SymbolEncoder::new(),
            above_level: [vec![0; mi_cols], vec![0; mi_cols], vec![0; mi_cols]],
            above_dc: [vec![0; mi_cols], vec![0; mi_cols], vec![0; mi_cols]],
            left_level: [vec![0; mi_rows], vec![0; mi_rows], vec![0; mi_rows]],
            left_dc: [vec![0; mi_rows], vec![0; mi_rows], vec![0; mi_rows]],
            mi_bsl: vec![0; mi_cols * mi_rows],
            mi_bsl_h: vec![0; mi_cols * mi_rows],
            above_partition: vec![0; mi_cols],
            left_partition: vec![0; mi_rows],
            mi_ymode: vec![0; mi_cols * mi_rows],
            mi_skip: vec![0; mi_cols * mi_rows],
            mi_segid: vec![0; mi_cols * mi_rows],
            mi_psize: vec![0; mi_cols * mi_rows],
            mi_pcolors: vec![[0u8; 8]; mi_cols * mi_rows],
            mi_dlf: vec![0; mi_cols * mi_rows],
            tx_log2: vec![2; mi_cols * mi_rows],
            tx_log2_h: vec![2; mi_cols * mi_rows],
            sb_r: 0,
            sb_c: 0,
            block_decoded: vec![0; BD_STRIDE * BD_STRIDE],
        }
    }

    /// `clear_block_decoded_flags` (§5.11.3) for the luma plane of the superblock at MI `(r, c)`:
    /// the row above and column left of the superblock (within the frame) are available; the
    /// interior is not yet decoded; the below-left corner is forced unavailable.
    fn clear_block_decoded(&mut self, r: usize, c: usize) {
        const SB4: isize = 16;
        let sb_width4 = (self.tile_c1 - c) as isize;
        let sb_height4 = (self.tile_r1 - r) as isize;
        for y in -1..=SB4 {
            for x in -1..=SB4 {
                // The row above the superblock and the column to its left (within the frame) are
                // already reconstructed; the interior is not yet decoded.
                let avail = (y < 0 && x < sb_width4) || (x < 0 && y < sb_height4);
                self.block_decoded[(y + 1) as usize * BD_STRIDE + (x + 1) as usize] = avail.into();
            }
        }
        self.block_decoded[(SB4 + 1) as usize * BD_STRIDE] = 0; // [sbSize4][-1] = 0
    }

    /// Reads a `BlockDecoded` flag at superblock-relative `(by, bx)` (both in `-1..=16`).
    fn block_decoded_at(&self, by: isize, bx: isize) -> bool {
        self.block_decoded[(by + 1) as usize * BD_STRIDE + (bx + 1) as usize] != 0
    }

    /// Encodes the tile and returns the symbol-coded bytes (`decode_tile`, §5.11.2) plus the
    /// reconstruction (lossy path).
    /// Signals the loop-restoration units (§5.11.57 `read_lr`) whose top-left falls in the superblock
    /// at MI `(r, c)`. Luma uses `RESTORE_WIENER` with the default reference filter — every coded
    /// coefficient delta is zero — and chroma is `RESTORE_NONE`. The unit grid is 256×256
    /// (`lr_unit_shift = 2`); the same filter is later applied by [`crate::filter::loop_restore_wiener_luma`].
    fn write_lr(&mut self, r: usize, c: usize) {
        const UNIT: usize = 256;
        // (offset, num_syms, k) for the three Wiener half-taps (§5.11.58 read_lr_unit).
        const TAPS: [(i32, i32, u32); 3] = [(5, 16, 1), (23, 32, 2), (17, 64, 3)];
        let unit_cols = ((self.width + UNIT / 2) / UNIT).max(1);
        let unit_rows = ((self.height + UNIT / 2) / UNIT).max(1);
        let row_start = (r * 4).div_ceil(UNIT);
        let row_end = unit_rows.min(((r + 16) * 4).div_ceil(UNIT));
        let col_start = (c * 4).div_ceil(UNIT);
        let col_end = unit_cols.min(((c + 16) * 4).div_ceil(UNIT));
        for _ in row_start..row_end {
            for _ in col_start..col_end {
                // restore_wiener = 1 (use Wiener), then vertical then horizontal half-taps.
                self.sym.encode_symbol(1, &cdf::RESTORE_WIENER);
                for _pass in 0..2 {
                    for (j, &(off, num_syms, k)) in TAPS.iter().enumerate() {
                        let v = crate::filter::WIENER_DEFAULT[j] + off;
                        encode_subexp_with_ref(&mut self.sym, v, num_syms, k, v);
                    }
                }
            }
        }
    }

    pub(crate) fn encode(mut self) -> (Vec<Vec<u8>>, Reconstruction) {
        const SB4: usize = 16; // 64×64 superblock in MI units
        let sb_cols = self.mi_cols.div_ceil(SB4);
        // Uniform tile spacing (§5.9.15): split a multi-superblock-wide frame into two tile columns
        // so the multi-tile framing is exercised. Each tile decodes independently — its symbol
        // contexts and `CurrentQIndex`/`DeltaLF` accumulators reset, and neighbour availability stops
        // at the tile edge (handled by `tile_c0`/`tile_x0`).
        let tile_cols = if sb_cols >= 2 { 2 } else { 1 };
        let size_sb = sb_cols.div_ceil(tile_cols);
        let mut tile_bytes: Vec<Vec<u8>> = Vec::with_capacity(tile_cols);
        for t in 0..tile_cols {
            let c_start = (t * size_sb * SB4).min(self.mi_cols);
            let c_end = ((t + 1) * size_sb * SB4).min(self.mi_cols);
            self.tile_c0 = c_start;
            self.tile_x0 = c_start * 4;
            self.tile_c1 = c_end;
            // Per-tile reset: a fresh range coder and the delta accumulators back to frame defaults.
            self.set_quant(i32::from(self.qindex));
            self.current_dlf = 0;
            let mut r = 0;
            while r < self.mi_rows {
                for plane in 0..3 {
                    self.left_level[plane].iter_mut().for_each(|v| *v = 0);
                    self.left_dc[plane].iter_mut().for_each(|v| *v = 0);
                }
                let mut c = c_start;
                while c < c_end {
                    self.sb_r = r;
                    self.sb_c = c;
                    // ReadDeltas is armed per superblock (§7.18); the first block then signals delta_q.
                    self.read_deltas = self.delta_q_present;
                    self.clear_block_decoded(r, c);
                    if self.qindex > 0 {
                        self.write_lr(r, c);
                    }
                    self.encode_partition(r, c, 64);
                    c += SB4;
                }
                r += SB4;
            }
            let sym = std::mem::replace(&mut self.sym, SymbolEncoder::new());
            tile_bytes.push(sym.finish());
        }
        // Deblocking loop filter (§7.14): a post-process on the final reconstruction (intra
        // prediction during encoding read the pre-filter samples, so this does not affect any
        // prediction). The lossless path is `CodedLossless`, where the filter is disabled.
        let mut planes = self.recon;
        let mut deblocked_luma = Vec::new();
        if self.qindex > 0 {
            crate::filter::deblock(
                &mut planes,
                self.coded_w,
                self.mi_cols,
                self.width,
                self.height,
                &self.tx_log2,
                &self.tx_log2_h,
                &self.mi_bsl,
                &self.mi_bsl_h,
                &self.mi_dlf,
                self.qindex,
            );
            // CDEF reads the deblocked reconstruction and produces a deringed one (§7.15). The
            // deblocked luma is retained for the loop-restoration stripe boundaries (§7.17); loop
            // restoration itself is applied by the caller after the optional superres upscale.
            deblocked_luma = planes[0].clone();
            planes = crate::filter::cdef(
                &planes,
                self.coded_w,
                &self.mi_skip,
                self.mi_cols,
                self.qindex,
            );
        }
        let recon = Reconstruction {
            planes,
            coded_w: self.coded_w,
            bit_depth: self.bit_depth,
            deblocked_luma,
        };
        (tile_bytes, recon)
    }

    /// Padded (edge-replicated) source sample of `plane` at coded-grid position `(x, y)`.
    fn sample(&self, plane: usize, x: usize, y: usize) -> i32 {
        let xx = x.min(self.width - 1);
        let yy = y.min(self.height - 1);
        i32::from(self.planes[plane][yy * self.width + xx])
    }

    fn encode_partition(&mut self, r: usize, c: usize, bw: usize) {
        if r >= self.mi_rows || c >= self.mi_cols {
            return;
        }
        let num4x4 = bw / 4;
        let half = num4x4 >> 1;
        let has_rows = r + half < self.mi_rows;
        let has_cols = c + half < self.mi_cols;
        let bsl = num4x4.trailing_zeros() as usize; // Mi_Width_Log2

        // `bp`: the chosen partition (0 NONE, 1 HORZ, 2 VERT, 3 SPLIT; `usize::MAX` = no partition
        // decoded, for the forced-NONE sub-8×8 blocks).
        let bp = if bw < 8 {
            usize::MAX
        } else if has_rows && has_cols {
            let ctx = self.partition_ctx(r, c, bsl);
            if self.qindex == 0 {
                self.sym.encode_symbol(0, partition_cdf(bsl, ctx)); // PARTITION_NONE
                0
            } else if block_exceeds_frame(r, c, num4x4, self.mi_rows, self.mi_cols) {
                // The block extends past the frame edge (only its top/left half is in-frame, which is
                // why `has_rows && has_cols` still hold). AV1 permits a frame-spanning PARTITION_NONE
                // here, but the lossy reconstruction buffer is the MI-frame size, so a single
                // block-size transform would write out of bounds. Force PARTITION_SPLIT — always a
                // legal choice in this branch — so the recursion descends until every coded transform
                // fits (a 4×4 leaf always fits); the out-of-frame leaves are skipped in `residual`.
                self.sym.encode_symbol(3, partition_cdf(bsl, ctx)); // PARTITION_SPLIT
                3
            } else if let Some(d) = self.decide_rect(r, c, bw) {
                // PARTITION_HORZ (1) / PARTITION_VERT (2): two rectangular halves, each its own mode
                // and one matching rectangular transform.
                self.sym.encode_symbol(d, partition_cdf(bsl, ctx));
                d
            } else if (8..=64).contains(&bw) && !self.should_split(r, c, bw) {
                // Lossy: an 8×8…64×64 region coded as a single PARTITION_NONE block + one
                // TX_8X8…TX_64X64 when it is smooth enough; otherwise split for per-block
                // mode/transform adaptation (each SPLIT halves, recursing down to 4×4 as needed).
                self.sym.encode_symbol(0, partition_cdf(bsl, ctx)); // PARTITION_NONE
                0
            } else {
                self.sym.encode_symbol(3, partition_cdf(bsl, ctx)); // PARTITION_SPLIT
                3
            }
        } else if has_cols {
            let ctx = self.partition_ctx(r, c, bsl);
            let cdf2 = split_or_horz_cdf(partition_cdf(bsl, ctx));
            self.sym.encode_symbol(1, &cdf2); // split
            3
        } else if has_rows {
            let ctx = self.partition_ctx(r, c, bsl);
            let cdf2 = split_or_vert_cdf(partition_cdf(bsl, ctx));
            self.sym.encode_symbol(1, &cdf2); // split
            3
        } else {
            3 // forced PARTITION_SPLIT, no symbol
        };

        // AbovePartitionContext/LeftPartitionContext update (§5.11.4): a terminal partition (NONE/
        // HORZ/VERT) writes its level's context; PARTITION_SPLIT writes only at 8×8 (its 4×4 children
        // decode no partition), otherwise its children update as they recurse. The forced-NONE sub-8×8
        // case (`bp = MAX`) decodes no partition, so it writes nothing.
        if bw >= 8 && bp != usize::MAX && (bp != 3 || bsl == 1) {
            self.update_partition_ctx(r, c, bsl, bp);
        }

        match bp {
            usize::MAX | 0 => self.encode_block(r, c, bw, bw),
            1 => {
                // PARTITION_HORZ: top then bottom half, each `bw × bw/2`.
                self.encode_block(r, c, bw, bw / 2);
                if r + half < self.mi_rows {
                    self.encode_block(r + half, c, bw, bw / 2);
                }
            }
            2 => {
                // PARTITION_VERT: left then right half, each `bw/2 × bw`.
                self.encode_block(r, c, bw / 2, bw);
                if c + half < self.mi_cols {
                    self.encode_block(r, c + half, bw / 2, bw);
                }
            }
            _ => {
                let h = bw / 2;
                self.encode_partition(r, c, h);
                self.encode_partition(r, c + half, h);
                self.encode_partition(r + half, c, h);
                self.encode_partition(r + half, c + half, h);
            }
        }
    }

    fn partition_ctx(&self, r: usize, c: usize, bsl: usize) -> usize {
        // §8.3.2: `above`/`left` are bit `4 - bl == bsl - 1` of the neighbour partition context.
        let bit = bsl - 1;
        let above = (self.above_partition[c] >> bit) & 1;
        // A tile's left edge has no left neighbour (left_partition holds the adjacent tile's value).
        let left = if c > self.tile_c0 {
            (self.left_partition[r] >> bit) & 1
        } else {
            0
        };
        usize::from(left) * 2 + usize::from(above)
    }

    /// Updates `AbovePartitionContext`/`LeftPartitionContext` over the `n4 × n4` MI cells of a block
    /// coded with terminal partition `bp` (0 = NONE, 1 = HORZ, 2 = VERT) at level `bsl`
    /// (`Mi_Width_Log2`). `PARTITION_SPLIT` does not update — its children write their own context.
    fn update_partition_ctx(&mut self, r: usize, c: usize, bsl: usize, bp: usize) {
        let bl = 5 - bsl; // block level: BL_128X128=0 … BL_8X8=4
        let n4 = 1usize << bsl;
        let (a, l) = (AL_PART_CTX[0][bl][bp], AL_PART_CTX[1][bl][bp]);
        for k in 0..n4 {
            if c + k < self.mi_cols {
                self.above_partition[c + k] = a;
            }
            if r + k < self.mi_rows {
                self.left_partition[r + k] = l;
            }
        }
    }

    /// Decides whether the lossy `bw × bw` region at MI `(r, c)` should split rather than be coded as
    /// a single `PARTITION_NONE` block. A textured region (large luma range over the source) splits so
    /// each sub-block can pick its own mode and transform; a smooth region stays whole, where one
    /// prediction and a single larger transform code it more compactly. This is a quality decision —
    /// either choice reconstructs bit-exactly — so the threshold is a deterministic placeholder
    /// (rate-distortion partition search is deferred).
    fn should_split(&self, r: usize, c: usize, bw: usize) -> bool {
        let n4 = bw / 4;
        // A 64×64 block that extends past the frame edge is split: the encoder only codes a
        // `PARTITION_NONE` 64×64 block when the whole superblock is on-screen (a partial 64×64 block
        // is valid but adds no value, and the smaller sub-blocks crop cleanly).
        if bw == 64 && (r + n4 > self.mi_rows || c + n4 > self.mi_cols) {
            return true;
        }
        // A palette-able block (few luma colors + flat chroma) is kept whole so it can use palette
        // mode, even though its luma range is large — splitting it would forfeit the palette coding.
        if self.decide_palette(r, c, bw).is_some() {
            return false;
        }
        self.luma_range(c * 4, r * 4, bw, bw) > 32
    }

    /// Updates the active dequantizers to `CurrentQIndex` — used when a `delta_q` changes it. Only the
    /// `dc_quant`/`ac_quant` step sizes follow `CurrentQIndex`; the coefficient-CDF context `qctx`
    /// stays at its frame value (`init_coeff_cdfs` derives it from `base_q_idx`, §8.3.2), so it is
    /// deliberately not touched here.
    fn set_quant(&mut self, qindex: i32) {
        self.current_qindex = qindex;
        self.dc_quant = dc_q(8, qindex);
        self.ac_quant = ac_q(8, qindex);
    }

    /// Sets the active dc/ac quantizers for a block to `get_qindex(segment)` (§7.12.2): the segment's
    /// `SEG_LVL_ALT_Q` delta is added to `CurrentQIndex` (the delta-Q accumulator, left unchanged), so
    /// each block can quantize at its own step size.
    fn apply_seg_quant(&mut self, segid: usize) {
        let alt = SEG_ALT_Q[segid].unwrap_or(0);
        let bq = (self.current_qindex + alt).clamp(0, 255);
        self.dc_quant = dc_q(8, bq);
        self.ac_quant = ac_q(8, bq);
    }

    /// `read_segment_id` (§5.11.9) after the skip flag (`SegIdPreSkip = 0`): derives the predicted
    /// segment id from the above/left/above-left neighbours. A skip block inherits the prediction (no
    /// bits); otherwise the encoder's deterministic assignment is coded as `neg_interleave` of the id
    /// relative to the prediction, under `Default_Segment_Id_Cdf[ctx]`. Returns the chosen id.
    fn signal_segment_id(&mut self, r: usize, c: usize, skip: bool) -> usize {
        let cols = self.mi_cols;
        let prev_ul = if r > 0 && c > self.tile_c0 {
            i32::from(self.mi_segid[(r - 1) * cols + (c - 1)])
        } else {
            -1
        };
        let prev_u = if r > 0 {
            i32::from(self.mi_segid[(r - 1) * cols + c])
        } else {
            -1
        };
        let prev_l = if c > self.tile_c0 {
            i32::from(self.mi_segid[r * cols + (c - 1)])
        } else {
            -1
        };
        let pred = if prev_u == -1 {
            prev_l.max(0)
        } else if prev_l == -1 || prev_ul == prev_u {
            prev_u
        } else {
            prev_l
        };
        if skip {
            return pred as usize; // inherited, no bits
        }
        // Deterministic assignment in 0..=LastActiveSegId; spatially varied so the three contexts and
        // both alt-Q segments are exercised. Any valid assignment reconstructs bit-exactly.
        let assigned = ((r + c) as i32) % (LAST_ACTIVE_SEG + 1);
        let ctx = if prev_ul < 0 {
            0
        } else if prev_ul == prev_u && prev_ul == prev_l {
            2
        } else if prev_ul == prev_u || prev_ul == prev_l || prev_u == prev_l {
            1
        } else {
            0
        };
        let diff = neg_interleave(assigned, pred, LAST_ACTIVE_SEG + 1);
        self.sym.encode_symbol(diff as usize, &cdf::SEGMENT_ID[ctx]);

        assigned as usize
    }

    /// `read_delta_qindex` (§5.11.12) for the first block of a superblock: signals a small per-SB
    /// `delta_q` (magnitude coded under `Default_Delta_Q_Cdf`, then a sign bit) and updates
    /// `CurrentQIndex = Clip3(1, 255, CurrentQIndex + delta)` (with `delta_q_res = 0`). The delta is a
    /// quality placeholder (any value reconstructs bit-exactly) chosen to oscillate around the frame
    /// quantizer so adjacent superblocks differ. Magnitude is kept ≤ 2, so the `DELTA_Q_SMALL` escape
    /// is never coded.
    fn signal_delta_q(&mut self) {
        let sb_idx = self.sb_r / 16 + self.sb_c / 16;
        let delta: i32 = match sb_idx % 3 {
            0 => 1,
            1 => -1,
            _ => 2,
        };
        let abs = delta.unsigned_abs() as usize;
        self.sym.encode_symbol(abs, &cdf::DELTA_Q);
        self.sym.encode_literal(u32::from(delta < 0), 1); // delta_q_sign_bit (1 ⇒ negative)
        let nq = (self.current_qindex + delta).clamp(1, 255);
        self.set_quant(nq);
    }

    /// `read_delta_lf` (§5.11.13, `delta_lf_multi = 0`) for the first block of a superblock: signals a
    /// small per-SB `delta_lf` (magnitude under `Default_Delta_Lf_Cdf`, then a sign bit when non-zero)
    /// and updates `CurrentDeltaLF = Clip3(-MAX_LOOP_FILTER, MAX_LOOP_FILTER, CurrentDeltaLF + delta)`.
    /// The delta is a non-negative `{0, 1}` placeholder (so the per-superblock loop-filter level only
    /// rises above the frame level — `loop_filter_level + DeltaLF` stays positive and the p-side-level
    /// fallback for a zero level is never needed). When the frame level is 0 the delta is forced to 0
    /// (the deblock stays disabled). Magnitude ≤ 2 ⇒ the `DELTA_LF_SMALL` escape is never coded.
    fn signal_delta_lf(&mut self) {
        let lf = i32::from(crate::filter::deblock_level(self.qindex));
        let delta: i32 = if lf == 0 {
            0
        } else {
            (self.sb_r / 16 + self.sb_c / 16) as i32 % 2
        };
        let abs = delta.unsigned_abs() as usize;
        self.sym.encode_symbol(abs, &cdf::DELTA_LF);
        if abs > 0 {
            self.sym.encode_literal(u32::from(delta < 0), 1); // delta_lf_sign_bit
        }
        self.current_dlf = (self.current_dlf + delta).clamp(-63, 63);
    }

    /// Context for the `skip` flag (§8.3.2): `aboveSkip + leftSkip` from the per-MI `mi_skip` map.
    fn skip_ctx(&self, r: usize, c: usize) -> usize {
        let above = r > 0 && self.mi_skip[(r - 1) * self.mi_cols + c] != 0;
        let left = c > self.tile_c0 && self.mi_skip[r * self.mi_cols + (c - 1)] != 0;
        usize::from(above) + usize::from(left)
    }

    /// Whether a lossy block at MI `(r, c)` can be coded with `skip = 1` (no residual): every plane is
    /// flat over the block and its `DC_PRED` from the reconstruction exactly equals that value, so the
    /// residual is identically zero. For such a block `skip = 1` and `skip = 0` reconstruct to the
    /// same samples (prediction), so signalling `skip` is a lossless bit saving. The luma mode a flat
    /// block selects is `DC_PRED` with no CfL / filter-intra, so the predicted samples equal the
    /// source — the chosen `skip` is therefore bit-exact regardless of the (deterministic) mode.
    fn block_is_skippable(&self, r: usize, c: usize, bw: usize) -> bool {
        let (sx, sy) = (c * 4, r * 4);
        for plane in 0..3 {
            let v0 = self.sample(plane, sx, sy);
            for i in 0..bw {
                for j in 0..bw {
                    if self.sample(plane, sx + j, sy + i) != v0 {
                        return false;
                    }
                }
            }
            if self.dc_pred(plane, sx, sy, bw, bw) != v0 {
                return false;
            }
        }
        true
    }

    /// Decides whether to code a lossy block at MI `(r, c)` with luma palette mode (§5.11.46): the
    /// luma block must have 2..=8 distinct colors, and (so the block can be coded `skip = 1` with the
    /// reconstruction being exactly the palette + DC chroma) every chroma plane must be flat and
    /// DC-predictable. Returns the sorted palette and the per-pixel index map. This is a quality
    /// decision; any choice reconstructs bit-exactly, so only the signaling has to be correct.
    fn decide_palette(&self, r: usize, c: usize, bw: usize) -> Option<PaletteBlock> {
        let (sx, sy) = (c * 4, r * 4);
        // Distinct luma colors (sorted).
        let mut set = [false; 256];
        let mut count = 0;
        for i in 0..bw {
            for j in 0..bw {
                let v = self.sample(0, sx + j, sy + i) as usize;
                if !set[v] {
                    set[v] = true;
                    count += 1;
                }
            }
        }
        if !(2..=8).contains(&count) {
            return None;
        }
        // Chroma must be flat and exactly DC-predictable (chroma residual identically 0 ⇒ skip = 1).
        for plane in 1..3 {
            let v0 = self.sample(plane, sx, sy);
            for i in 0..bw {
                for j in 0..bw {
                    if self.sample(plane, sx + j, sy + i) != v0 {
                        return None;
                    }
                }
            }
            if self.dc_pred(plane, sx, sy, bw, bw) != v0 {
                return None;
            }
        }
        let colors: Vec<u8> = (0..256).filter(|&v| set[v]).map(|v| v as u8).collect();
        let mut index_map = vec![0u8; bw * bw];
        for i in 0..bw {
            for j in 0..bw {
                let v = self.sample(0, sx + j, sy + i) as u8;
                index_map[i * bw + j] = colors.binary_search(&v).unwrap_or(0) as u8;
            }
        }

        Some(PaletteBlock { colors, index_map })
    }

    /// `get_palette_cache` (§5.11.46) for luma: the sorted, deduplicated merge of the above (only
    /// when the block is not at the top of its 64-superblock) and left blocks' palettes.
    fn palette_cache(&self, r: usize, c: usize) -> Vec<u8> {
        let above_n = if !r.is_multiple_of(16) {
            self.mi_psize[(r - 1) * self.mi_cols + c] as usize
        } else {
            0
        };
        let left_n = if c > self.tile_c0 {
            self.mi_psize[r * self.mi_cols + (c - 1)] as usize
        } else {
            0
        };
        let blank = [0u8; 8];
        let above = if above_n > 0 {
            &self.mi_pcolors[(r - 1) * self.mi_cols + c]
        } else {
            &blank
        };
        let left = if left_n > 0 {
            &self.mi_pcolors[r * self.mi_cols + (c - 1)]
        } else {
            &blank
        };
        let mut cache: Vec<u8> = Vec::new();
        let push = |cache: &mut Vec<u8>, v: u8| {
            if cache.last() != Some(&v) {
                cache.push(v);
            }
        };
        let (mut ai, mut li) = (0, 0);
        while ai < above_n && li < left_n {
            let (ac, lc) = (above[ai], left[li]);
            if lc < ac {
                push(&mut cache, lc);
                li += 1;
            } else {
                push(&mut cache, ac);
                ai += 1;
                if lc == ac {
                    li += 1;
                }
            }
        }
        while ai < above_n {
            push(&mut cache, above[ai]);
            ai += 1;
        }
        while li < left_n {
            push(&mut cache, left[li]);
            li += 1;
        }
        cache
    }

    /// `ns(n)` literal coding (§4.10.7) of `val ∈ [0, n)`.
    fn encode_ns(&mut self, val: usize, n: usize) {
        if n <= 1 {
            return;
        }
        let w = n.ilog2() + 1;
        let m = (1usize << w) - n;
        if val < m {
            self.sym.encode_literal(val as u32, w - 1);
        } else {
            let coded = val + m;
            self.sym.encode_literal((coded >> 1) as u32, w - 1);
            self.sym.encode_literal((coded & 1) as u32, 1);
        }
    }

    /// Signals the luma palette colors (§5.11.46): the cache-usage flags (all 0 — the encoder reuses
    /// no cached color) followed by every palette color as new — the first raw, the rest delta-coded.
    fn signal_palette_colors(&mut self, colors: &[u8], cache_n: usize) {
        let psize = colors.len();
        // use_palette_color_cache_y flags: 0 for each cached color (none reused), while idx < psize.
        // Since none are used, idx stays 0 and all `cache_n` flags are emitted.
        for _ in 0..cache_n {
            self.sym.encode_literal(0, 1);
        }
        // New colors: the first is raw 8-bit; the rest are positive deltas coded in `palette_bits`.
        self.sym.encode_literal(u32::from(colors[0]), 8);
        if psize > 1 {
            self.sym.encode_literal(3, 2); // palette_num_extra_bits_y = 3 ⇒ palette_bits = 8 (always fits)
            let mut palette_bits = 5 + 3u32;
            for idx in 1..psize {
                let delta = i32::from(colors[idx]) - i32::from(colors[idx - 1]); // ≥ 1 (sorted distinct)
                self.sym.encode_literal((delta - 1) as u32, palette_bits);
                let range = 256 - i32::from(colors[idx]) - 1;
                palette_bits = palette_bits.min(ceil_log2(range.max(1) as usize));
            }
        }
    }

    /// Signals the luma `color_index_map_y` (§5.11.49): the top-left index via `ns()`, then the
    /// remaining indices in anti-diagonal (wavefront) order, each as the position of its color in the
    /// neighbour-derived `ColorOrder`, coded under the size/context palette-color CDF.
    fn signal_palette_tokens(&mut self, index_map: &[u8], bw: usize, psize: usize) {
        self.encode_ns(index_map[0] as usize, psize);
        for i in 1..(2 * bw - 1) {
            let j_start = i.min(bw - 1);
            let j_end = i.saturating_sub(bw - 1);
            let mut j = j_start;
            loop {
                let (rr, cc) = (i - j, j);
                let (order, ctx) = palette_color_context(index_map, bw, rr, cc, psize);
                let actual = index_map[rr * bw + cc] as usize;
                let sym = order.iter().position(|&x| x == actual).unwrap_or(0);
                self.sym.encode_symbol(sym, palette_color_cdf(psize, ctx));
                if j == j_end {
                    break;
                }
                j -= 1;
            }
        }
    }

    /// Reconstructs a palette block: luma is `palette[ColorMapY]`, chroma is the (flat) DC prediction.
    /// No residual is coded (the block is `skip = 1`), and the neighbour level/dc contexts are reset.
    fn recon_palette(&mut self, r: usize, c: usize, bw: usize, pal: &PaletteBlock) {
        let (sx, sy) = (c * 4, r * 4);
        for i in 0..bw {
            for j in 0..bw {
                let idx = pal.index_map[i * bw + j] as usize;
                self.recon[0][(sy + i) * self.coded_w + (sx + j)] = u16::from(pal.colors[idx]);
            }
        }
        for plane in 1..3 {
            let dc = clip_pixel(self.dc_pred(plane, sx, sy, bw, bw), self.bit_depth);
            for i in 0..bw {
                for j in 0..bw {
                    self.recon[plane][(sy + i) * self.coded_w + (sx + j)] = dc;
                }
            }
        }
        for plane in 0..3 {
            self.set_ctx(plane, sx >> 2, sy >> 2, bw / 4, bw / 4, 0, 0);
        }
    }

    /// Source-luma range (max − min) over the `bw × bw` block at coded `(sx, sy)`. Used by the
    /// partition and transform-depth heuristics.
    fn luma_range(&self, sx: usize, sy: usize, w: usize, h: usize) -> i32 {
        let mut lo = i32::MAX;
        let mut hi = i32::MIN;
        for i in 0..h {
            for j in 0..w {
                let v = self.sample(0, sx + j, sy + i);
                lo = lo.min(v);
                hi = hi.max(v);
            }
        }
        hi - lo
    }

    /// Decides whether to code a square `bw × bw` region (16×16 or 32×32, fully on-screen) as two
    /// rectangular halves: `Some(1)` PARTITION_HORZ (top/bottom `bw × bw/2`) or `Some(2)` PARTITION_VERT
    /// (left/right `bw/2 × bw`). Chosen when the whole block is textured (would otherwise split) but a
    /// single horizontal or vertical edge makes both halves smooth — so each half gets one mode/
    /// transform. A quality decision (every choice reconstructs bit-exactly); the threshold is a
    /// deterministic placeholder. `None` falls through to the NONE/SPLIT decision.
    fn decide_rect(&self, r: usize, c: usize, bw: usize) -> Option<usize> {
        if bw != 16 && bw != 32 {
            return None;
        }
        let n4 = bw / 4;
        if r + n4 > self.mi_rows || c + n4 > self.mi_cols {
            return None; // a partial block crops cleanly as smaller squares
        }
        let (sx, sy, hp) = (c * 4, r * 4, bw / 2);
        if self.luma_range(sx, sy, bw, bw) <= 32 {
            return None; // smooth enough to keep whole (PARTITION_NONE)
        }
        let horz_ok =
            self.luma_range(sx, sy, bw, hp) <= 32 && self.luma_range(sx, sy + hp, bw, hp) <= 32;
        let vert_ok =
            self.luma_range(sx, sy, hp, bw) <= 32 && self.luma_range(sx + hp, sy, hp, bw) <= 32;
        if horz_ok {
            Some(1)
        } else if vert_ok {
            Some(2)
        } else {
            None
        }
    }

    /// Context for the `tx_depth` CDF (§8.3.2): `(aboveTxW ≥ bw) + (leftTxH ≥ bw)`, where the
    /// neighbour transform width/height come from the per-MI `tx_log2` map (0 when unavailable).
    fn tx_depth_ctx(&self, r: usize, c: usize, bw: usize, bh: usize) -> usize {
        let above_w = if r > 0 {
            1usize << self.tx_log2[(r - 1) * self.mi_cols + c]
        } else {
            0
        };
        let left_h = if c > self.tile_c0 {
            1usize << self.tx_log2_h[r * self.mi_cols + (c - 1)]
        } else {
            0
        };
        usize::from(above_w >= bw) + usize::from(left_h >= bh)
    }

    /// Picks the luma `tx_depth` (0..=`max_depth`, itself ≤ `MAX_TX_DEPTH = 2`) for a lossy ≥8×8
    /// block: smoother blocks keep one block-size transform, more textured ones split the transform
    /// (one prediction mode, several sub-transforms). This is a quality decision — every depth
    /// reconstructs bit-exactly — so the range thresholds are a deterministic placeholder.
    fn select_tx_depth(&self, sx: usize, sy: usize, bw: usize, max_depth: usize) -> usize {
        let range = self.luma_range(sx, sy, bw, bw);
        let depth = if range > 24 {
            2
        } else if range > 12 {
            1
        } else {
            0
        };
        depth.min(max_depth)
    }

    fn encode_block(&mut self, r: usize, c: usize, bw: usize, bh: usize) {
        let n4 = bw / 4;
        let n4h = bh / 4;
        let bsl = n4.trailing_zeros() as u8;
        let bsl_h = n4h.trailing_zeros() as u8;
        // A rectangular block (from PARTITION_HORZ/VERT) is coded as one matching rectangular transform
        // (OPTION A); it uses a SMOOTH-family non-directional mode (so no palette/filter-intra/CfL) and
        // never `skip = 1` (its residual, possibly all-zero, is always coded).
        let is_rect = bw != bh;
        // TX_MODE_LARGEST ⇒ the transform spans the whole block. The lossy path codes an 8×8 or 16×16
        // block as a single TX_8X8 / TX_16X16; lossy 4×4 and lossless use 4×4 transforms.
        let lossy_large = self.qindex > 0 && bw.min(bh) >= 8;

        // intra_frame_mode_info: skip=0 (ctx 0), then y_mode and uv_mode. The lossless path always
        // predicts DC_PRED; the lossy path searches the non-directional luma modes (plus directional
        // and filter-intra) and signals the choice with the above/left mode context.
        // palette_mode_info / read_skip. A palette block (luma 2..=8 colors + flat chroma) is coded
        // `skip = 1`: the reconstruction is the palette (luma) plus DC chroma, so no residual is
        // needed. Otherwise a block whose residual is identically zero (flat + DC-predicted) is also
        // `skip = 1`. The skip context is the above/left skip flags.
        let palette = if self.qindex > 0 && !is_rect && (8..=32).contains(&bw) {
            self.decide_palette(r, c, bw)
        } else {
            None
        };
        let skip =
            palette.is_some() || (self.qindex > 0 && !is_rect && self.block_is_skippable(r, c, bw));

        let sctx = self.skip_ctx(r, c);
        self.sym.encode_symbol(usize::from(skip), &cdf::SKIP[sctx]);
        // intra_segment_id / read_segment_id (§5.11.9): with segmentation on, code the block's
        // segment id right after the skip flag (`SegIdPreSkip = 0`). A skip block inherits the
        // predicted id with no bits.
        let segid = if self.qindex > 0 {
            self.signal_segment_id(r, c, skip)
        } else {
            0
        };
        // read_delta_qindex (§5.11.12): the first coded block of each superblock signals delta_q and
        // updates `CurrentQIndex`; later blocks in the superblock inherit it. (read_cdef codes no bits:
        // cdef_bits = 0.) A block that fills the whole superblock (`MiSize == sbSize`, i.e. a 64×64
        // block) and is `skip` codes neither delta — `read_delta_qindex`/`read_delta_lf` return early.
        if self.read_deltas {
            if !(bw == 64 && bh == 64 && skip) {
                self.signal_delta_q();
                self.signal_delta_lf();
            }
            self.read_deltas = false;
        }
        // Per-block quantizer get_qindex(segment) = Clip3(0, 255, CurrentQIndex + alt_q) (§7.12.2):
        // only the dc/ac step sizes follow the segment; `CurrentQIndex` stays the delta-Q accumulator.
        if self.qindex > 0 {
            self.apply_seg_quant(segid);
        }
        let (y_mode, angle_delta, filter_intra) = if palette.is_some() || self.qindex == 0 {
            // Palette requires YMode == DC_PRED (and no filter-intra); lossless also forces DC_PRED.
            (DC_PRED, 0i8, None)
        } else if is_rect {
            // A rectangular block picks a SMOOTH-family non-directional mode (never DC, so no palette/
            // filter-intra signaling follows, and never directional, so prediction stays square-free).
            (self.select_luma_mode_rect(c * 4, r * 4, bw, bh), 0, None)
        } else if lossy_large {
            self.select_luma_mode_nxn(c * 4, r * 4, bw)
        } else {
            let (m, fi) = self.select_luma_mode(c * 4, r * 4);
            (m, 0, fi)
        };
        let amode = cdf::INTRA_MODE_CONTEXT[if r > 0 {
            usize::from(self.mi_ymode[(r - 1) * self.mi_cols + c])
        } else {
            usize::from(DC_PRED)
        }];
        let lmode = cdf::INTRA_MODE_CONTEXT[if c > self.tile_c0 {
            usize::from(self.mi_ymode[r * self.mi_cols + (c - 1)])
        } else {
            usize::from(DC_PRED)
        }];
        self.sym
            .encode_symbol(usize::from(y_mode), &cdf::INTRA_FRAME_Y_MODE[amode][lmode]);
        // intra_angle_info_y (§5.11.42): for MiSize ≥ BLOCK_8X8 with a directional YMode, the fine
        // angle `AngleDeltaY ∈ [-3, 3]` is signaled (biased by MAX_ANGLE_DELTA = 3) under the
        // per-mode `Angle_Delta_Cdf`. 4×4 blocks never reach this (MiSize < BLOCK_8X8 ⇒ delta = 0).
        if bw >= 8 && is_directional(y_mode) {
            let sym = (i32::from(angle_delta) + 3) as usize;
            self.sym
                .encode_symbol(sym, &cdf::ANGLE_DELTA[(y_mode - V_PRED) as usize]);
        }
        // uv_mode. Its CDF is indexed by the luma mode. `is_cfl_allowed` is `MiSize == BLOCK_4X4`
        // when Lossless, else `Max(w, h) <= 32` — so CfL is allowed for the lossy 4×4 and 8×8 blocks
        // and the lossless 4×4 blocks. When allowed, the encoder picks chroma-from-luma if it beats
        // plain DC_PRED, then emits read_cfl_alphas; otherwise uv_mode = DC_PRED.
        let cfl_allowed = if self.qindex == 0 {
            bw == 4
        } else {
            bw.max(bh) <= 32
        };
        let cfl = if palette.is_some() {
            None // a palette block's chroma is DC_PRED (no CfL)
        } else if self.qindex > 0 && cfl_allowed && !is_rect {
            self.select_cfl(c * 4, r * 4, bw)
        } else {
            None // CfL is square-only here; a rectangular block's chroma is plain DC_PRED
        };
        let ym = usize::from(y_mode);
        if cfl_allowed {
            let uv = if cfl.is_some() { UV_CFL_PRED } else { DC_PRED };
            self.sym
                .encode_symbol(usize::from(uv), &cdf::UV_MODE_CFL_ALLOWED[ym]);
            if let Some((au, av)) = cfl {
                self.emit_cfl_alphas(au, av);
            }
        } else {
            self.sym.encode_symbol(0, &cdf::UV_MODE_CFL_NOT_ALLOWED[ym]);
        }

        // palette_mode_info (§5.11.46): with allow_screen_content_tools an 8×8..64×64 block signals
        // `has_palette_y` (when YMode == DC_PRED) and `has_palette_uv` (when UVMode == DC_PRED). When
        // luma palette is selected, the palette size, the cache-usage flags, and the colors follow.
        // (Palette is only *chosen* for ≤32 — `palette` is `None` at 64 — but the flag is still coded.)
        if self.qindex > 0 && (8..=64).contains(&bw) {
            if y_mode == DC_PRED {
                let bctx = bw.trailing_zeros() as usize + bh.trailing_zeros() as usize - 6; // Mi_W_Log2 + Mi_H_Log2 - 2
                let above = r > 0 && self.mi_psize[(r - 1) * self.mi_cols + c] > 0;
                let left = c > self.tile_c0 && self.mi_psize[r * self.mi_cols + (c - 1)] > 0;
                let pctx = usize::from(above) + usize::from(left);
                self.sym.encode_symbol(
                    usize::from(palette.is_some()),
                    &cdf::PALETTE_Y_MODE[bctx][pctx],
                );
                if let Some(pal) = &palette {
                    // palette_size_y_minus_2, then the colors (cache flags + new colors).
                    self.sym
                        .encode_symbol(pal.colors.len() - 2, &cdf::PALETTE_Y_SIZE[bctx]);
                    let cache_n = self.palette_cache(r, c).len();
                    let colors = pal.colors.clone();
                    self.signal_palette_colors(&colors, cache_n);
                }
            }
            // UVMode == DC_PRED; the context is `(PaletteSizeY > 0)`. No chroma palette is used.
            if cfl.is_none() {
                let uctx = usize::from(palette.is_some());
                self.sym.encode_symbol(0, &cdf::PALETTE_UV_MODE[uctx]);
            }
        }

        // filter_intra_mode_info (§5.11.24): a DC_PRED luma block ≤ 32px with no palette
        // (`PaletteSizeY == 0`) signals use_filter_intra, under the block-size CDF row. A 64×64 block
        // (`Max(w, h) > 32`) does not signal it.
        if self.qindex > 0 && y_mode == DC_PRED && palette.is_none() && bw <= 32 && bh <= 32 {
            let fi_cdf: &[u16] = match bw {
                4 => &cdf::FILTER_INTRA_4X4,
                8 => &cdf::FILTER_INTRA_8X8,
                16 => &cdf::FILTER_INTRA_16X16,
                _ => &cdf::FILTER_INTRA_32X32,
            };
            self.sym
                .encode_symbol(usize::from(filter_intra.is_some()), fi_cdf);
            if let Some(fi) = filter_intra {
                self.sym
                    .encode_symbol(usize::from(fi), &cdf::FILTER_INTRA_MODE);
            }
        }

        // palette_tokens (§5.11.49): the color_index_map_y, coded after mode_info and before
        // read_block_tx_size.
        if let Some(pal) = &palette {
            let idx_map = pal.index_map.clone();
            self.signal_palette_tokens(&idx_map, bw, pal.colors.len());
        }

        // read_block_tx_size (§5.11.16): under TX_MODE_SELECT a lossy ≥8×8 luma block signals
        // `tx_depth`, choosing a luma transform `bw >> tx_depth` (square, ≥ 4×4). Chroma never signals
        // — for 4:4:4 it always uses the block-size transform (`Max_Tx_Size_Rect`). The CDF is keyed by
        // block size (`Max_Tx_Depth`); the context is `(aboveTxW ≥ bw) + (leftTxH ≥ bw)`.
        let luma_tx = if is_rect {
            // One rectangular transform fills the block (tx_depth = 0). The CDF is keyed by the
            // bounding square (`txSzSqrUp = Max(bw, bh)`); the context is `(aboveTxW ≥ bw) +
            // (leftTxH ≥ bh)`, exactly as for a square block of that size.
            let ctx = self.tx_depth_ctx(r, c, bw, bh);
            let cdf: &[u16] = match bw.max(bh) {
                16 => &cdf::TX_SIZE_16X16[ctx],
                _ => &cdf::TX_SIZE_32X32[ctx],
            };
            self.sym.encode_symbol(0, cdf);
            rect_tx(bw, bh)
        } else if lossy_large {
            let max_depth = if bw == 8 { 1 } else { 2 };
            let tx_depth = self.select_tx_depth(c * 4, r * 4, bw, max_depth);
            let ctx = self.tx_depth_ctx(r, c, bw, bw);
            let cdf: &[u16] = match bw {
                8 => &cdf::TX_SIZE_8X8[ctx],
                16 => &cdf::TX_SIZE_16X16[ctx],
                32 => &cdf::TX_SIZE_32X32[ctx],
                _ => &cdf::TX_SIZE_64X64[ctx],
            };

            self.sym.encode_symbol(tx_depth, cdf);
            square_tx(bw >> tx_depth)
        } else {
            TxSize::Tx4x4
        };

        // Per-MI bookkeeping: block size, luma mode (for neighbour contexts), and the luma
        // transform-size log2 — consumed by the deblocking loop filter (which derives the chroma tx
        // size from `mi_bsl` since 4:4:4 chroma uses the block-size transform).
        let txl = luma_tx.log2_width() as u8;
        let txl_h = luma_tx.log2_height() as u8;
        let (psize, pcolors) = match &palette {
            Some(pal) => {
                let mut buf = [0u8; 8];
                buf[..pal.colors.len()].copy_from_slice(&pal.colors);
                (pal.colors.len() as u8, buf)
            }
            None => (0u8, [0u8; 8]),
        };
        for y in 0..n4h {
            for x in 0..n4 {
                let (rr, cc) = (r + y, c + x);
                if rr < self.mi_rows && cc < self.mi_cols {
                    self.mi_bsl[rr * self.mi_cols + cc] = bsl;
                    self.mi_bsl_h[rr * self.mi_cols + cc] = bsl_h;
                    self.mi_ymode[rr * self.mi_cols + cc] = y_mode;
                    self.tx_log2[rr * self.mi_cols + cc] = txl;
                    self.tx_log2_h[rr * self.mi_cols + cc] = txl_h;
                    self.mi_skip[rr * self.mi_cols + cc] = u8::from(skip);
                    self.mi_psize[rr * self.mi_cols + cc] = psize;
                    self.mi_pcolors[rr * self.mi_cols + cc] = pcolors;
                    self.mi_segid[rr * self.mi_cols + cc] = segid as u8;
                    self.mi_dlf[rr * self.mi_cols + cc] = self.current_dlf as i8;
                }
            }
        }

        // A palette block reconstructs directly (luma = palette[ColorMapY], chroma = DC) with no
        // residual, then marks its luma cells decoded so later directional predictions see them.
        if let Some(pal) = palette {
            self.recon_palette(r, c, bw, &pal);
            for ty in 0..bw / 4 {
                for tx in 0..bw / 4 {
                    let by = (r + ty) as isize - self.sb_r as isize;
                    let bx = (c + tx) as isize - self.sb_c as isize;
                    if (0..16).contains(&by) && (0..16).contains(&bx) {
                        self.block_decoded[(by + 1) as usize * BD_STRIDE + (bx + 1) as usize] = 1;
                    }
                }
            }
            return;
        }

        // residual(): per plane. Luma uses the block's intra mode and signaled transform size (a
        // raster of `luma_tx` sub-transforms, each predicted + reconstructed in turn); 4:4:4 chroma is
        // DC_PRED (plus the CfL term) over the block-size transform — but chroma never uses TX_64X64,
        // so a 64×64 block's chroma is a 2×2 raster of TX_32X32. Luma (plane 0) is fully reconstructed
        // before chroma, so the chroma CfL reads finalized luma recon.
        let chroma_tx = if is_rect {
            // 4:4:4 chroma uses the same rectangular transform as luma (one transform fills the block).
            rect_tx(bw, bh)
        } else {
            match bw {
                8 => TxSize::Tx8x8,
                16 => TxSize::Tx16x16,
                _ => TxSize::Tx32x32,
            }
        };
        for plane in 0..3 {
            let pred = Pred {
                mode: if plane == 0 { y_mode } else { DC_PRED },
                angle_delta: if plane == 0 { angle_delta } else { 0 },
                filter_intra: if plane == 0 { filter_intra } else { None },
                cfl_alpha: match (plane, cfl) {
                    (1, Some((au, _))) => Some(au),
                    (2, Some((_, av))) => Some(av),
                    _ => None,
                },
            };
            let (ptx, pw, ph) = if plane == 0 {
                (luma_tx, luma_tx.width(), luma_tx.height())
            } else if lossy_large {
                // Chroma transform size — the chroma block size capped at 32 (no TX_64X64); the
                // residual loop steps by `pw`/`ph`, so e.g. a 64×64 block's chroma forms a 2×2 raster.
                (chroma_tx, chroma_tx.width(), chroma_tx.height())
            } else {
                (TxSize::Tx4x4, 4, 4)
            };
            let mut sy = r * 4;
            while sy < r * 4 + bh {
                let mut sx = c * 4;
                while sx < c * 4 + bw {
                    if sx < self.coded_w && sy < self.coded_h {
                        self.transform_block(plane, sx, sy, bw, ptx, pred, skip);
                    }
                    // BlockDecoded (§5.11.34) is updated after each *transform* block (not the whole
                    // block), so a later luma sub-transform's directional prediction sees the
                    // above-right / below-left siblings just reconstructed. Luma-grid only (chroma is
                    // never directional); marked on the luma pass.
                    if plane == 0 {
                        for ty in 0..ph / 4 {
                            for tx in 0..pw / 4 {
                                let by = (sy / 4 + ty) as isize - self.sb_r as isize;
                                let bx = (sx / 4 + tx) as isize - self.sb_c as isize;
                                if (0..16).contains(&by) && (0..16).contains(&bx) {
                                    self.block_decoded
                                        [(by + 1) as usize * BD_STRIDE + (bx + 1) as usize] = 1;
                                }
                            }
                        }
                    }
                    sx += pw;
                }
                sy += ph;
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn transform_block(
        &mut self,
        plane: usize,
        sx: usize,
        sy: usize,
        block_w: usize,
        tx_size: TxSize,
        desc: Pred,
        skip: bool,
    ) {
        // Lossless: forced 4×4 Walsh–Hadamard, DC prediction, source-as-recon (M0 path, unchanged).
        if self.qindex == 0 {
            let pred = self.predict_4x4(plane, sx, sy, desc.mode);
            let mut res = [0i32; 16];
            for i in 0..4 {
                for j in 0..4 {
                    res[i * 4 + j] = self.sample(plane, sx + j, sy + i) - pred[i * 4 + j];
                }
            }
            let quant = gamut_dsp::fwht4x4(&res);
            self.code_coeffs(
                plane,
                sx >> 2,
                sy >> 2,
                block_w,
                TxSize::Tx4x4,
                &quant,
                1,
                0,
            );
            return;
        }

        // Lossy, square transform of side `n` (4 or 8). Predict from the reconstruction buffer,
        // forward-transform + quantize, code, then reconstruct exactly as the decoder will and store
        // back. Because the recon is `pred + inverse(dequant(levels))` and the decoder runs the same
        // inverse on the same levels, the result is bit-exact for whichever transform type is signaled.
        let (tw, th) = (tx_size.width(), tx_size.height());
        let pred: Vec<i32> = match desc.filter_intra {
            Some(fi) if plane == 0 => self.predict_filter_intra(plane, sx, sy, tw, fi),
            _ => {
                let mut p = self.predict_intra(plane, sx, sy, desc.mode, tw, th, desc.angle_delta);
                if let Some(alpha) = desc.cfl_alpha {
                    self.apply_cfl(&mut p, sx, sy, alpha, tw);
                }
                p
            }
        };
        // skip = 1: no residual is coded; the reconstruction is the (clipped) prediction, and the
        // neighbour level/dc contexts are reset to 0 (`reset_block_context`, §5.11.10).
        if skip {
            for (i, prow) in pred.chunks_exact(tw).enumerate() {
                for (j, &pv) in prow.iter().enumerate() {
                    self.recon[plane][(sy + i) * self.coded_w + (sx + j)] =
                        clip_pixel(pv, self.bit_depth);
                }
            }
            self.set_ctx(plane, sx >> 2, sy >> 2, tw / 4, th / 4, 0, 0);
            return;
        }
        let mut res = vec![0i32; tw * th];
        for i in 0..th {
            for j in 0..tw {
                res[i * tw + j] = self.sample(plane, sx + j, sy + i) - pred[i * tw + j];
            }
        }
        // A square luma transform searches the reduced intra tx-type set; chroma and every rectangular
        // transform use DCT_DCT (its set is DCT-only for ≥32, and DCT is chosen for the ≤16 rect sizes).
        let (tx, tx_sym, levels) = if plane == 0 && tw == th {
            self.select_tx_type(&res, tx_size)
        } else {
            (
                TxType::DctDct,
                1,
                self.quantize_tx(&res, tx_size, TxType::DctDct),
            )
        };
        // intraDir (§9.4) keys the 16×16 transform-type CDF: the filter-intra mode's mapped direction
        // when filter-intra is used, else YMode. Ignored for 4×4/8×8 (uniform CDF) and for chroma.
        let intra_dir = match desc.filter_intra {
            Some(fi) => usize::from(FILTER_INTRA_MODE_TO_INTRA_DIR[fi as usize]),
            None => usize::from(desc.mode),
        };
        self.code_coeffs(
            plane,
            sx >> 2,
            sy >> 2,
            block_w,
            tx_size,
            &levels,
            tx_sym,
            intra_dir,
        );

        // dqDenom (§7.12.3) divides the dequantized coefficient by the transform size class. A
        // conformant decoder applies it, so the encoder reconstruction must.
        let dq_denom = tx_size.dq_denom();
        let mut dq = vec![0i32; tw * th];
        if tw == 64 && th == 64 {
            // Re-expand the 32-stride coded levels into the top-left 32×32 of the 64-stride array the
            // inverse transform reads (the remaining coefficients stay zero).
            for i in 0..32 {
                for j in 0..32 {
                    let q = if i == 0 && j == 0 {
                        self.dc_quant
                    } else {
                        self.ac_quant
                    };
                    dq[i * 64 + j] = dequant(levels[i * 32 + j], q, dq_denom, 8);
                }
            }
        } else {
            for (i, &lvl) in levels.iter().enumerate() {
                let q = if i == 0 { self.dc_quant } else { self.ac_quant };
                dq[i] = dequant(lvl, q, dq_denom, 8);
            }
        }
        let resid = inverse_transform_2d(&dq, tx_size, tx, 8);
        for i in 0..th {
            for j in 0..tw {
                let v = clip_pixel(pred[i * tw + j] + resid[i * tw + j], self.bit_depth);
                self.recon[plane][(sy + i) * self.coded_w + (sx + j)] = v;
            }
        }
    }

    /// Forward-transforms and quantizes a square residual under transform type `tx`, returning the
    /// quantized coefficient levels (DC uses `dc_quant`, AC uses `ac_quant`). Coefficients are
    /// pre-scaled by `dqDenom` so that `dequant(level, q, dqDenom)` recovers the coefficient (the
    /// decoder divides by `dqDenom` for 32×32; §7.12.3) — without it a 32×32 block would reconstruct
    /// at half residual amplitude. This is a quality choice; bit-exactness is unaffected (both the
    /// encoder reconstruction and the decoder dequantize the same coded levels with the same divisor).
    fn quantize_tx(&self, res: &[i32], tx_size: TxSize, tx: TxType) -> Vec<i32> {
        let coeff = forward_transform_2d(res, tx_size, tx);
        let denom = tx_size.dq_denom();
        if tx_size.width() == 64 {
            // TX_64X64 codes only its top-left 32×32 sub-block; compact those into the 32-stride
            // 1024-coefficient layout the coding/scan expect (the rest are zeroed by the transform).
            let mut levels = vec![0i32; 32 * 32];
            for i in 0..32 {
                for j in 0..32 {
                    let q = if i == 0 && j == 0 {
                        self.dc_quant
                    } else {
                        self.ac_quant
                    };
                    levels[i * 32 + j] = quantize(coeff[i * 64 + j] * denom, q);
                }
            }
            return levels;
        }
        let mut levels = vec![0i32; coeff.len()];
        for (i, &c) in coeff.iter().enumerate() {
            let q = if i == 0 { self.dc_quant } else { self.ac_quant };
            levels[i] = quantize(c * denom, q);
        }
        levels
    }

    /// Selects a luma transform type from `TX_SET_INTRA_2` by a simple coded-cost proxy (sum of
    /// `1 + |level|` over non-zero coefficients), preferring `DCT_DCT` on ties. Returns the type,
    /// its `Tx_Type_Intra_Inv_Set2` symbol, and the quantized levels. The choice is a quality
    /// decision (any valid type round-trips bit-exactly); only the *signaling* must be correct. A
    /// 32×32 block is `TX_SET_DCTONLY` (§5.11.48), so it is forced to `DCT_DCT` (no type is signaled).
    fn select_tx_type(&self, res: &[i32], tx_size: TxSize) -> (TxType, usize, Vec<i32>) {
        if tx_size.width() >= 32 {
            return (
                TxType::DctDct,
                1,
                self.quantize_tx(res, tx_size, TxType::DctDct),
            );
        }
        let cost = |levels: &[i32]| -> i32 {
            levels
                .iter()
                .map(|&l| if l == 0 { 0 } else { 1 + l.abs() })
                .sum()
        };
        // Start from DCT_DCT (symbol 1) so it wins ties; the rest of TX_SET_INTRA_2 follows.
        let mut best = (
            TxType::DctDct,
            1usize,
            self.quantize_tx(res, tx_size, TxType::DctDct),
        );
        let mut best_cost = cost(&best.2);
        for (tx, sym) in [
            (TxType::AdstAdst, 2usize),
            (TxType::AdstDct, 3),
            (TxType::DctAdst, 4),
            (TxType::Idtx, 0),
        ] {
            let levels = self.quantize_tx(res, tx_size, tx);
            let c = cost(&levels);
            if c < best_cost {
                best_cost = c;
                best = (tx, sym, levels);
            }
        }
        best
    }

    /// DC intra prediction value for a 4×4 at coded position `(sx, sy)` (§7.11.2.5). In the lossless
    /// path neighbours are the (padded) source (which equals the reconstruction); in the lossy path
    /// they are the reconstruction buffer, matching the decoder exactly.
    fn dc_avg(&self, plane: usize, sx: usize, sy: usize) -> i32 {
        self.dc_pred(plane, sx, sy, 4, 4)
    }

    /// `DC_PRED` value for an `n × n` block at coded `(sx, sy)` (§7.11.2.5): the rounded average of
    /// the available `n` above and `n` left neighbours (both sides ⇒ `Round2(sum, log2(n) + 1)`; one
    /// side ⇒ `Round2(sum, log2(n))`; neither ⇒ `1 << (BitDepth - 1)`).
    fn dc_pred(&self, plane: usize, sx: usize, sy: usize, w: usize, h: usize) -> i32 {
        let nb = |x: usize, y: usize| -> i32 {
            if self.qindex > 0 {
                i32::from(self.recon[plane][y * self.coded_w + x])
            } else {
                self.sample(plane, x, y)
            }
        };
        let have_above = sy > 0;
        let have_left = sx > self.tile_x0;
        // §7.11.2.5 DC: average the `w` above + `h` left samples (a plain integer divide, since for a
        // rectangular block `w + h` is not a power of two; for square blocks this is the usual shift).
        match (have_above, have_left) {
            (true, true) => {
                let mut s = 0;
                for k in 0..w {
                    s += nb(sx + k, sy - 1);
                }
                for k in 0..h {
                    s += nb(sx - 1, sy + k);
                }
                (s + ((w + h) as i32 >> 1)) / (w + h) as i32
            }
            (false, true) => {
                let mut s = 0;
                for k in 0..h {
                    s += nb(sx - 1, sy + k);
                }
                (s + (h as i32 >> 1)) / h as i32
            }
            (true, false) => {
                let mut s = 0;
                for k in 0..w {
                    s += nb(sx + k, sy - 1);
                }
                (s + (w as i32 >> 1)) / w as i32
            }
            (false, false) => 128, // 1 << (BitDepth - 1)
        }
    }

    /// Builds the §7.11.2.1 reference samples for a 4×4 transform block at coded `(sx, sy)`:
    /// `AboveRow[0..8]` and `LeftCol[0..8]` (the `w + h` samples), plus the top-left corner
    /// (`AboveRow[-1] == LeftCol[-1]`). Reads the reconstruction buffer with the spec's availability
    /// fallbacks: when a neighbour is missing the side defaults to `127`/`129` (or replicates the
    /// orthogonal edge); samples past the block width replicate the last valid one unless the
    /// above-right / below-left 4×4 has been decoded (`BlockDecoded`, §5.11.34), in which case they
    /// are real samples.
    fn reference_4x4(&self, plane: usize, sx: usize, sy: usize) -> ([i32; 8], [i32; 8], i32) {
        let nb = |x: usize, y: usize| -> i32 { i32::from(self.recon[plane][y * self.coded_w + x]) };
        let have_above = sy > 0;
        let have_left = sx > self.tile_x0;

        // Above-right / below-left availability (§5.11.34) for the directional zone-1/3 angles.
        let (by, bx) = (
            sy as isize / 4 - self.sb_r as isize,
            sx as isize / 4 - self.sb_c as isize,
        );
        let have_above_right = self.block_decoded_at(by - 1, bx + 1);
        let have_below_left = self.block_decoded_at(by + 1, bx - 1);
        let (max_x, max_y) = (self.coded_w - 1, self.coded_h - 1);

        let mut above = [127i32; 8]; // (1 << (BitDepth-1)) - 1 when neither neighbour exists
        if have_above {
            let above_limit = max_x.min(sx + if have_above_right { 8 } else { 4 } - 1);
            for (i, a) in above.iter_mut().enumerate() {
                *a = nb(above_limit.min(sx + i), sy - 1);
            }
        } else if have_left {
            above = [nb(sx - 1, sy); 8]; // CurrFrame[y][x-1]
        }

        let mut left = [129i32; 8]; // (1 << (BitDepth-1)) + 1 when neither neighbour exists
        if have_left {
            let left_limit = max_y.min(sy + if have_below_left { 8 } else { 4 } - 1);
            for (i, l) in left.iter_mut().enumerate() {
                *l = nb(sx - 1, left_limit.min(sy + i));
            }
        } else if have_above {
            left = [nb(sx, sy - 1); 8]; // CurrFrame[y-1][x]
        }

        let top_left = if have_above && have_left {
            nb(sx - 1, sy - 1)
        } else if have_above {
            nb(sx, sy - 1)
        } else if have_left {
            nb(sx - 1, sy)
        } else {
            128 // 1 << (BitDepth-1)
        };
        (above, left, top_left)
    }

    /// Builds the `n × n` recursive filter-intra prediction (§7.11.2.3) for luma at coded `(sx, sy)`
    /// under `filter_intra_mode`. The block is tiled into `w4 = n/4` by `h2 = n/2` overlapping 4×2
    /// sub-blocks, processed top-to-bottom then left-to-right so each sub-block can filter the already
    /// predicted samples of the one above and to its left. Each output 4×2 weights a 7-sample
    /// neighbourhood `p` by [`cdf::INTRA_FILTER_TAPS`] and scales by `INTRA_FILTER_SCALE_BITS = 4`
    /// (`Round2Signed`). Only `AboveRow[-1..n-1]` / `LeftCol[-1..n-1]` are read, so no above-right /
    /// below-left extension is needed. Used only on the lossy path, where `enable_filter_intra = 1`.
    fn predict_filter_intra(
        &self,
        plane: usize,
        sx: usize,
        sy: usize,
        n: usize,
        fi_mode: u8,
    ) -> Vec<i32> {
        let (above, left, top_left) = self.reference_basic(plane, sx, sy, n, n);
        let above_row = |k: i32| -> i32 { if k < 0 { top_left } else { above[k as usize] } };
        let left_col = |k: i32| -> i32 { if k < 0 { top_left } else { left[k as usize] } };
        let taps = &cdf::INTRA_FILTER_TAPS[fi_mode as usize];
        let (w4, h2, nn) = ((n >> 2) as i32, (n >> 1) as i32, n as i32);
        let mut pred = vec![0i32; n * n];
        // For each 4×2 sub-block (i2 = rows of 2, j4 = columns of 4), in this exact order so the
        // dependencies on the sub-block above (`(i2<<1)-1`) and to the left (`(j4<<2)-1`) are ready.
        for i2 in 0..h2 {
            for j4 in 0..w4 {
                // p[0..6]: the 7 neighbouring samples (§7.11.2.3). Top row reads AboveRow; left column
                // reads LeftCol; the interior reads already-predicted samples.
                let mut p = [0i32; 7];
                for (i, pi) in p.iter_mut().enumerate() {
                    let i = i as i32;
                    *pi = if i < 5 {
                        if i2 == 0 {
                            above_row((j4 << 2) + i - 1)
                        } else if j4 == 0 && i == 0 {
                            left_col((i2 << 1) - 1)
                        } else {
                            pred[(((i2 << 1) - 1) * nn + (j4 << 2) + i - 1) as usize]
                        }
                    } else if j4 == 0 {
                        left_col((i2 << 1) + i - 5)
                    } else {
                        pred[(((i2 << 1) + i - 5) * nn + (j4 << 2) - 1) as usize]
                    };
                }
                for i1 in 0..2i32 {
                    for j1 in 0..4i32 {
                        let row = ((i1 << 2) + j1) as usize;
                        let mut pr = 0i32;
                        for (k, &tap) in taps[row].iter().enumerate() {
                            pr += i32::from(tap) * p[k];
                        }
                        let idx = (((i2 << 1) + i1) * nn + (j4 << 2) + j1) as usize;
                        pred[idx] = round2_signed(pr, 4).clamp(0, 255);
                    }
                }
            }
        }
        pred
    }

    /// Builds the 4×4 intra prediction for `plane` at coded position `(sx, sy)` under `mode`
    /// (§7.11.2). `DC_PRED` replicates [`Self::dc_avg`]; the other supported modes (non-directional
    /// `PAETH`/`SMOOTH`/`SMOOTH_V`/`SMOOTH_H` and directional `V`/`H`/`D135`/`D113`/`D157`) are only
    /// used for lossy luma and read the reconstruction buffer, applying the spec's reference-sample
    /// availability fallbacks. Returns the prediction row-major. With `enable_intra_edge_filter = 0`
    /// and `angle_delta = 0` (4×4), the supported modes never read past the 4 above / 4 left samples
    /// plus the top-left corner, so above-right / below-left extension is not needed.
    fn predict_4x4(&self, plane: usize, sx: usize, sy: usize, mode: u8) -> [i32; 16] {
        if mode == DC_PRED {
            return [self.dc_avg(plane, sx, sy); 16];
        }
        let (above, left, top_left) = self.reference_4x4(plane, sx, sy);

        // Directional derivatives (`Dr_Intra_Derivative`). Zone 1 (pAngle < 90): dx = Dr[pAngle].
        // Zone 2 (90 < pAngle < 180): dx = Dr[180 - pAngle], dy = Dr[pAngle - 90]. Zone 3
        // (pAngle > 180): dy = Dr[270 - pAngle]. Unused by the non-directional / cardinal modes.
        let (dx, dy): (i32, i32) = match mode {
            D45_PRED => (64, 0),    // Dr[45]
            D67_PRED => (27, 0),    // Dr[67]
            D135_PRED => (64, 64),  // Dr[45], Dr[45]
            D113_PRED => (27, 151), // Dr[67], Dr[23]
            D157_PRED => (151, 27), // Dr[23], Dr[67]
            D203_PRED => (0, 27),   // Dr[67]
            _ => (0, 0),
        };

        let w = cdf::SM_WEIGHTS_4X4;
        let mut pred = [0i32; 16];
        for i in 0..4 {
            for j in 0..4 {
                pred[i * 4 + j] = match mode {
                    V_PRED => above[j],
                    H_PRED => left[i],
                    D45_PRED | D67_PRED => {
                        // §7.11.2.4 zone 1 (pAngle < 90): interpolate along the (extended) above row.
                        let (ii, jj) = (i as i32, j as i32);
                        let idx = (ii + 1) * dx;
                        let base = (idx >> 6) + jj;
                        let max_base_x = 4 + 4 - 1; // (w + h - 1)
                        if base < max_base_x {
                            let shift = (idx >> 1) & 0x1F;
                            (above[base as usize] * (32 - shift)
                                + above[(base + 1) as usize] * shift
                                + 16)
                                >> 5
                        } else {
                            above[max_base_x as usize]
                        }
                    }
                    D203_PRED => {
                        // §7.11.2.4 zone 3 (pAngle > 180): interpolate along the (extended) left col.
                        let (ii, jj) = (i as i32, j as i32);
                        let idx = (jj + 1) * dy;
                        let base = (idx >> 6) + ii;
                        let shift = (idx >> 1) & 0x1F;
                        (left[base as usize] * (32 - shift)
                            + left[(base + 1) as usize] * shift
                            + 16)
                            >> 5
                    }
                    D135_PRED | D113_PRED | D157_PRED => {
                        // §7.11.2.4 zone 2 (upsample disabled): interpolate along the above row,
                        // falling back to the left column when the ray leaves the top edge. Indices
                        // -1 map to the top-left corner; the chosen angles keep them within [-1, 3].
                        let (ii, jj) = (i as i32, j as i32);
                        let idx = (jj << 6) - (ii + 1) * dx;
                        let base = idx >> 6;
                        if base >= -1 {
                            let shift = (idx >> 1) & 0x1F;
                            let a0 = if base < 0 {
                                top_left
                            } else {
                                above[base as usize]
                            };
                            let a1 = above[(base + 1) as usize];
                            (a0 * (32 - shift) + a1 * shift + 16) >> 5 // Round2(_, 5)
                        } else {
                            let idx2 = (ii << 6) - (jj + 1) * dy;
                            let base2 = idx2 >> 6;
                            let shift = (idx2 >> 1) & 0x1F;
                            let l0 = if base2 < 0 {
                                top_left
                            } else {
                                left[base2 as usize]
                            };
                            let l1 = left[(base2 + 1) as usize];
                            (l0 * (32 - shift) + l1 * shift + 16) >> 5 // Round2(_, 5)
                        }
                    }
                    PAETH_PRED => {
                        let base = above[j] + left[i] - top_left;
                        let p_left = (base - left[i]).abs();
                        let p_top = (base - above[j]).abs();
                        let p_tl = (base - top_left).abs();
                        if p_left <= p_top && p_left <= p_tl {
                            left[i]
                        } else if p_top <= p_tl {
                            above[j]
                        } else {
                            top_left
                        }
                    }
                    SMOOTH_PRED => {
                        let v = w[i] * above[j]
                            + (256 - w[i]) * left[3]
                            + w[j] * left[i]
                            + (256 - w[j]) * above[3];
                        (v + 256) >> 9 // Round2(_, 9)
                    }
                    SMOOTH_V_PRED => {
                        let v = w[i] * above[j] + (256 - w[i]) * left[3];
                        (v + 128) >> 8 // Round2(_, 8)
                    }
                    _ => {
                        // SMOOTH_H_PRED
                        let v = w[j] * left[i] + (256 - w[j]) * above[3];
                        (v + 128) >> 8 // Round2(_, 8)
                    }
                };
            }
        }
        pred
    }

    /// Builds the `n × n` intra prediction for `plane` at coded `(sx, sy)` under `mode`. 4×4 blocks
    /// dispatch to [`Self::predict_4x4`] (`angle_delta` is always 0 there — never signaled for
    /// `MiSize < BLOCK_8X8`). 8×8 and 16×16 blocks use the non-directional predictor for
    /// `DC`/`SMOOTH*`/`PAETH` and the general directional process (§7.11.2.4) — fine-tuned by
    /// `angle_delta` — otherwise.
    #[allow(clippy::too_many_arguments)]
    fn predict_intra(
        &self,
        plane: usize,
        sx: usize,
        sy: usize,
        mode: u8,
        w: usize,
        h: usize,
        angle_delta: i8,
    ) -> Vec<i32> {
        // Directional prediction and the 4×4 fast path are square-only; a rectangular transform block
        // only ever uses the non-directional modes (the encoder never selects a directional mode for a
        // rectangular partition, so `angle_delta` is irrelevant there).
        if w != h {
            return self.predict_nondir(plane, sx, sy, w, h, mode);
        }
        let n = w;
        // Directional modes go through the general process so the block's `angle_delta` is honored —
        // including a 4×4 sub-transform of a ≥8×8 directional block under TX_MODE_SELECT (where
        // `angle_delta` may be non-zero). For `n == 4, angle_delta == 0` this matches `predict_4x4`
        // exactly (pinned by `general_directional_matches_4x4_path`).
        if is_directional(mode) {
            return self.predict_directional(plane, sx, sy, n, mode, angle_delta);
        }
        if n == 4 {
            return self.predict_4x4(plane, sx, sy, mode).to_vec();
        }
        self.predict_nondir(plane, sx, sy, n, n, mode)
    }

    /// Builds the §7.11.2.2 reference samples for an `n × n` directional block: `AboveRow[0..2n]`,
    /// `LeftCol[0..2n]`, and the top-left corner (`AboveRow[-1] == LeftCol[-1]`). The extended
    /// (`> n`) samples are real reconstruction only when the above-right / below-left `n × n` region
    /// has been decoded (`BlockDecoded`, §5.11.34, stepped by `n / 4` MI cells); otherwise they
    /// replicate the last in-block edge sample. With `enable_intra_edge_filter = 0` no edge filtering
    /// or upsampling is applied. Generalizes [`Self::reference_4x4`] (which is the `n == 4` case).
    fn reference_directional(
        &self,
        plane: usize,
        sx: usize,
        sy: usize,
        n: usize,
    ) -> (Vec<i32>, Vec<i32>, i32) {
        let nb = |x: usize, y: usize| -> i32 { i32::from(self.recon[plane][y * self.coded_w + x]) };
        let have_above = sy > 0;
        let have_left = sx > self.tile_x0;
        let step = (n / 4) as isize;
        let (by, bx) = (
            sy as isize / 4 - self.sb_r as isize,
            sx as isize / 4 - self.sb_c as isize,
        );
        let have_above_right = self.block_decoded_at(by - 1, bx + step);
        let have_below_left = self.block_decoded_at(by + step, bx - 1);
        let (max_x, max_y) = (self.coded_w - 1, self.coded_h - 1);

        let mut above = vec![127i32; 2 * n];
        if have_above {
            let above_limit = max_x.min(sx + if have_above_right { 2 * n } else { n } - 1);
            for (i, a) in above.iter_mut().enumerate() {
                *a = nb(above_limit.min(sx + i), sy - 1);
            }
        } else if have_left {
            above = vec![nb(sx - 1, sy); 2 * n];
        }

        let mut left = vec![129i32; 2 * n];
        if have_left {
            let left_limit = max_y.min(sy + if have_below_left { 2 * n } else { n } - 1);
            for (i, l) in left.iter_mut().enumerate() {
                *l = nb(sx - 1, left_limit.min(sy + i));
            }
        } else if have_above {
            left = vec![nb(sx, sy - 1); 2 * n];
        }

        let top_left = if have_above && have_left {
            nb(sx - 1, sy - 1)
        } else if have_above {
            nb(sx, sy - 1)
        } else if have_left {
            nb(sx - 1, sy)
        } else {
            128
        };
        (above, left, top_left)
    }

    /// Directional intra prediction (§7.11.2.4) for an `n × n` block, with `pAngle = Mode_To_Angle +
    /// angle_delta * ANGLE_STEP` and `upsampleAbove = upsampleLeft = 0` (the intra edge filter is
    /// disabled). Cardinal angles (`90`/`180`) copy a single edge; zone 1 (`< 90`) interpolates along
    /// the extended above row; zone 2 (`90 < · < 180`) interpolates along the above row, falling back
    /// to the left column past the top edge; zone 3 (`> 180`) interpolates along the extended left
    /// column. `Dr_Intra_Derivative` supplies `dx`/`dy`; each interpolation is `Round2(_, 5)`.
    fn predict_directional(
        &self,
        plane: usize,
        sx: usize,
        sy: usize,
        n: usize,
        mode: u8,
        angle_delta: i8,
    ) -> Vec<i32> {
        let p_angle = cdf::MODE_TO_ANGLE[mode as usize] + i32::from(angle_delta) * 3;
        let (above, left, top_left) = self.reference_directional(plane, sx, sy, n);
        let above_row = |k: i32| -> i32 { if k < 0 { top_left } else { above[k as usize] } };
        let left_col = |k: i32| -> i32 { if k < 0 { top_left } else { left[k as usize] } };
        let mut pred = vec![0i32; n * n];
        for i in 0..n as i32 {
            for j in 0..n as i32 {
                let v = if p_angle == 90 {
                    above[j as usize]
                } else if p_angle == 180 {
                    left[i as usize]
                } else if p_angle < 90 {
                    // Zone 1: ray sweeps the above row (extended to the above-right).
                    let dx = cdf::DR_INTRA_DERIVATIVE[p_angle as usize];
                    let idx = (i + 1) * dx;
                    let base = (idx >> 6) + j;
                    let max_base_x = (2 * n - 1) as i32; // (w + h - 1)
                    if base < max_base_x {
                        let shift = (idx >> 1) & 0x1F;
                        (above[base as usize] * (32 - shift)
                            + above[(base + 1) as usize] * shift
                            + 16)
                            >> 5
                    } else {
                        above[max_base_x as usize]
                    }
                } else if p_angle < 180 {
                    // Zone 2: ray sweeps the above row, falling back to the left column past the top.
                    let dx = cdf::DR_INTRA_DERIVATIVE[(180 - p_angle) as usize];
                    let dy = cdf::DR_INTRA_DERIVATIVE[(p_angle - 90) as usize];
                    let idx = (j << 6) - (i + 1) * dx;
                    let base = idx >> 6;
                    if base >= -1 {
                        let shift = (idx >> 1) & 0x1F;
                        (above_row(base) * (32 - shift) + above_row(base + 1) * shift + 16) >> 5
                    } else {
                        let idx2 = (i << 6) - (j + 1) * dy;
                        let base2 = idx2 >> 6;
                        let shift = (idx2 >> 1) & 0x1F;
                        (left_col(base2) * (32 - shift) + left_col(base2 + 1) * shift + 16) >> 5
                    }
                } else {
                    // Zone 3: ray sweeps the left column (extended to the below-left).
                    let dy = cdf::DR_INTRA_DERIVATIVE[(270 - p_angle) as usize];
                    let idx = (j + 1) * dy;
                    let base = (idx >> 6) + i;
                    let shift = (idx >> 1) & 0x1F;
                    (left[base as usize] * (32 - shift) + left[(base + 1) as usize] * shift + 16)
                        >> 5
                };
                pred[(i * n as i32 + j) as usize] = v;
            }
        }
        pred
    }

    /// Builds the §7.11.2.1 reference samples for a non-directional `n × n` block: `AboveRow[0..n]`,
    /// `LeftCol[0..n]`, and the top-left corner, with the availability fallbacks (`127`/`129`/edge
    /// replication). The supported non-directional modes never read above-right / below-left, so the
    /// references stop at `n` samples (no `BlockDecoded` extension).
    fn reference_basic(
        &self,
        plane: usize,
        sx: usize,
        sy: usize,
        w: usize,
        h: usize,
    ) -> (Vec<i32>, Vec<i32>, i32) {
        let nb = |x: usize, y: usize| -> i32 { i32::from(self.recon[plane][y * self.coded_w + x]) };
        let have_above = sy > 0;
        let have_left = sx > self.tile_x0;
        let (max_x, max_y) = (self.coded_w - 1, self.coded_h - 1);

        let mut above = vec![127i32; w];
        if have_above {
            for (i, a) in above.iter_mut().enumerate() {
                *a = nb(max_x.min(sx + i), sy - 1);
            }
        } else if have_left {
            above = vec![nb(sx - 1, sy); w];
        }

        let mut left = vec![129i32; h];
        if have_left {
            for (i, l) in left.iter_mut().enumerate() {
                *l = nb(sx - 1, max_y.min(sy + i));
            }
        } else if have_above {
            left = vec![nb(sx, sy - 1); h];
        }

        let top_left = if have_above && have_left {
            nb(sx - 1, sy - 1)
        } else if have_above {
            nb(sx, sy - 1)
        } else if have_left {
            nb(sx - 1, sy)
        } else {
            128
        };
        (above, left, top_left)
    }

    /// Non-directional `n × n` intra prediction (§7.11.2): `DC_PRED`, `PAETH_PRED`, and the three
    /// `SMOOTH` modes, using the `n`-wide smooth weights (`Sm_Weights_Tx_NxN`). Reads the
    /// reconstruction buffer. Used for the lossy 8×8 and 16×16 non-directional luma blocks.
    fn predict_nondir(
        &self,
        plane: usize,
        sx: usize,
        sy: usize,
        w: usize,
        h: usize,
        mode: u8,
    ) -> Vec<i32> {
        if mode == DC_PRED {
            return vec![self.dc_pred(plane, sx, sy, w, h); w * h];
        }
        let (above, left, top_left) = self.reference_basic(plane, sx, sy, w, h);
        // SMOOTH weights are per axis: `sw` (horizontal, indexed by column) and `sh` (vertical, by
        // row). For a square block `sw == sh`; for a rectangular block they differ in length.
        let sw = sm_weights(w);
        let sh = sm_weights(h);
        let mut pred = vec![0i32; w * h];
        for i in 0..h {
            for j in 0..w {
                pred[i * w + j] = match mode {
                    PAETH_PRED => {
                        let base = above[j] + left[i] - top_left;
                        let p_left = (base - left[i]).abs();
                        let p_top = (base - above[j]).abs();
                        let p_tl = (base - top_left).abs();
                        if p_left <= p_top && p_left <= p_tl {
                            left[i]
                        } else if p_top <= p_tl {
                            above[j]
                        } else {
                            top_left
                        }
                    }
                    SMOOTH_PRED => {
                        let v = sh[i] * above[j]
                            + (256 - sh[i]) * left[h - 1]
                            + sw[j] * left[i]
                            + (256 - sw[j]) * above[w - 1];
                        (v + 256) >> 9
                    }
                    SMOOTH_V_PRED => {
                        let v = sh[i] * above[j] + (256 - sh[i]) * left[h - 1];
                        (v + 128) >> 8
                    }
                    _ => {
                        // SMOOTH_H_PRED
                        let v = sw[j] * left[i] + (256 - sw[j]) * above[w - 1];
                        (v + 128) >> 8
                    }
                };
            }
        }
        pred
    }

    /// Picks the lossy luma prediction minimizing SAD against the source 4×4 at `(sx, sy)`. Returns
    /// the `YMode` to signal and, when recursive filter-intra (§7.11.2.3) beats every regular mode,
    /// the chosen `filter_intra_mode` (signaled as `YMode = DC_PRED` + `use_filter_intra = 1`).
    /// Regular modes win ties (and `DC_PRED` wins among them), so the cheaper signaling is preferred.
    /// The choice is a quality decision; every option reconstructs bit-exactly, so only the
    /// signaling must be correct.
    fn select_luma_mode(&self, sx: usize, sy: usize) -> (u8, Option<u8>) {
        let sad = |pred: &[i32]| -> i32 {
            let mut s = 0;
            for i in 0..4 {
                for j in 0..4 {
                    s += (self.sample(0, sx + j, sy + i) - pred[i * 4 + j]).abs();
                }
            }
            s
        };
        let mut best_mode = DC_PRED;
        let mut best_sad = sad(&self.predict_4x4(0, sx, sy, DC_PRED));
        for &mode in &LUMA_MODES[1..] {
            let s = sad(&self.predict_4x4(0, sx, sy, mode));
            if s < best_sad {
                best_sad = s;
                best_mode = mode;
            }
        }
        // Recursive filter-intra (luma, DC_PRED blocks only). It wins only on a strict improvement,
        // so the extra `use_filter_intra` signaling is spent only when it pays off.
        let mut best_fi: Option<u8> = None;
        let mut best_fi_sad = i32::MAX;
        for fi in 0..5u8 {
            let s = sad(&self.predict_filter_intra(0, sx, sy, 4, fi));
            if s < best_fi_sad {
                best_fi_sad = s;
                best_fi = Some(fi);
            }
        }
        if best_fi_sad < best_sad {
            (DC_PRED, best_fi)
        } else {
            (best_mode, None)
        }
    }

    /// Picks the lossy luma mode for a rectangular `w × h` block minimizing SAD against the source.
    /// It searches only the non-`DC` non-directional modes (`SMOOTH`/`SMOOTH_V`/`SMOOTH_H`/`PAETH`):
    /// staying off `DC_PRED` means no `palette`/`use_filter_intra` flags follow, and staying off the
    /// directional modes keeps prediction within the rectangular non-directional path. Every option
    /// reconstructs bit-exactly, so this is purely a quality choice.
    fn select_luma_mode_rect(&self, sx: usize, sy: usize, w: usize, h: usize) -> u8 {
        let sad = |pred: &[i32]| -> i32 {
            let mut s = 0;
            for i in 0..h {
                for j in 0..w {
                    s += (self.sample(0, sx + j, sy + i) - pred[i * w + j]).abs();
                }
            }
            s
        };
        let mut best = SMOOTH_PRED;
        let mut best_sad = sad(&self.predict_nondir(0, sx, sy, w, h, SMOOTH_PRED));
        for &mode in &[SMOOTH_V_PRED, SMOOTH_H_PRED, PAETH_PRED] {
            let s = sad(&self.predict_nondir(0, sx, sy, w, h, mode));
            if s < best_sad {
                best_sad = s;
                best = mode;
            }
        }
        best
    }

    /// Picks the lossy luma mode for an `n × n` (8×8 or 16×16) block minimizing SAD against the
    /// source. It searches the non-directional modes (`DC`/`SMOOTH*`/`PAETH`), the eight directional
    /// modes each over `angle_delta ∈ [-3, 3]`, and (for `DC_PRED`) the five recursive filter-intra
    /// modes. Returns the `YMode`, its `AngleDeltaY` (0 for non-directional), and the chosen
    /// `filter_intra_mode` when filter-intra strictly wins. Regular modes win ties (and
    /// `DC_PRED`/`angle_delta = 0` win among them), so cheaper signaling is preferred. Every option
    /// reconstructs bit-exactly; this is purely a quality decision, so only the signaling must match.
    fn select_luma_mode_nxn(&self, sx: usize, sy: usize, n: usize) -> (u8, i8, Option<u8>) {
        let sad = |pred: &[i32]| -> i32 {
            let mut s = 0;
            for i in 0..n {
                for j in 0..n {
                    s += (self.sample(0, sx + j, sy + i) - pred[i * n + j]).abs();
                }
            }
            s
        };
        let mut best = (DC_PRED, 0i8);
        let mut best_sad = sad(&self.predict_nondir(0, sx, sy, n, n, DC_PRED));
        for &mode in &[SMOOTH_PRED, SMOOTH_V_PRED, SMOOTH_H_PRED, PAETH_PRED] {
            let s = sad(&self.predict_nondir(0, sx, sy, n, n, mode));
            if s < best_sad {
                best_sad = s;
                best = (mode, 0);
            }
        }
        for &mode in &DIRECTIONAL_MODES {
            for ad in -3..=3i8 {
                let s = sad(&self.predict_directional(0, sx, sy, n, mode, ad));
                if s < best_sad {
                    best_sad = s;
                    best = (mode, ad);
                }
            }
        }
        // Recursive filter-intra (signaled as DC_PRED + use_filter_intra); strict improvement only.
        let mut best_fi: Option<u8> = None;
        let mut best_fi_sad = i32::MAX;
        for fi in 0..5u8 {
            let s = sad(&self.predict_filter_intra(0, sx, sy, n, fi));
            if s < best_fi_sad {
                best_fi_sad = s;
                best_fi = Some(fi);
            }
        }
        if best_fi_sad < best_sad {
            (DC_PRED, 0, best_fi)
        } else {
            (best.0, best.1, None)
        }
    }

    /// Adds the chroma-from-luma high-frequency term to a 4×4 chroma DC prediction in place
    /// (§7.11.5). For 4:4:4 the subsampled luma `L[i][j]` is just the reconstructed luma sample
    /// (`× 8`, i.e. 3 fractional bits); `lumaAvg = Round2(ΣL, 4)`. Each chroma sample becomes
    /// `Clip1(dc + Round2Signed(alpha * (L - lumaAvg), 6))`. `alpha == 0` is a no-op (plain DC).
    fn apply_cfl(&self, pred: &mut [i32], sx: usize, sy: usize, alpha: i32, n: usize) {
        let mut l = vec![0i32; n * n];
        let mut sum = 0i32;
        for i in 0..n {
            for j in 0..n {
                let v = i32::from(self.recon[0][(sy + i) * self.coded_w + (sx + j)]) << 3;
                l[i * n + j] = v;
                sum += v;
            }
        }
        // lumaAvg = Round2(ΣL, Tx_Width_Log2 + Tx_Height_Log2) = Round2(ΣL, 2 * log2(n)).
        let shift = 2 * n.trailing_zeros();
        let luma_avg = (sum + (1 << (shift - 1))) >> shift;
        for (p, &lv) in pred.iter_mut().zip(&l) {
            *p = (*p + round2_signed(alpha * (lv - luma_avg), 6)).clamp(0, 255);
        }
    }

    /// Picks chroma-from-luma alphas for the 4×4 chroma blocks at `(sx, sy)` by minimizing per-plane
    /// SAD against the source chroma, or returns `None` when plain `DC_PRED` is at least as good for
    /// both planes (so the cheaper `uv_mode = DC_PRED` is signaled). The alpha search uses the
    /// *source* luma as a proxy for the reconstruction (a quality decision; the signaled alpha is
    /// applied to the true reconstruction in [`Self::apply_cfl`], so the result is bit-exact either
    /// way). Returns `(CflAlphaU, CflAlphaV)`, each in `-16..=16` and not both zero.
    fn select_cfl(&self, sx: usize, sy: usize, n: usize) -> Option<(i32, i32)> {
        // Source luma high-frequency (matching apply_cfl's reconstructed-luma formula).
        let mut l = vec![0i32; n * n];
        let mut sum = 0i32;
        for i in 0..n {
            for j in 0..n {
                let v = self.sample(0, sx + j, sy + i) << 3;
                l[i * n + j] = v;
                sum += v;
            }
        }
        let shift = 2 * n.trailing_zeros();
        let luma_avg = (sum + (1 << (shift - 1))) >> shift;

        let best_alpha = |plane: usize| -> i32 {
            let dc = self.dc_pred(plane, sx, sy, n, n);
            let sad = |alpha: i32| -> i32 {
                let mut s = 0;
                for i in 0..n {
                    for j in 0..n {
                        let pred = (dc + round2_signed(alpha * (l[i * n + j] - luma_avg), 6))
                            .clamp(0, 255);
                        s += (self.sample(plane, sx + j, sy + i) - pred).abs();
                    }
                }
                s
            };
            // alpha 0 (plain DC) is the baseline; CflAlpha magnitudes are 1..=16, either sign.
            let mut best = 0i32;
            let mut best_sad = sad(0);
            for mag in 1..=16 {
                for &a in &[mag, -mag] {
                    let s = sad(a);
                    if s < best_sad {
                        best_sad = s;
                        best = a;
                    }
                }
            }
            best
        };

        let au = best_alpha(1);
        let av = best_alpha(2);
        if au == 0 && av == 0 {
            None
        } else {
            Some((au, av))
        }
    }

    /// Emits `read_cfl_alphas` (§5.11.45): the joint `cfl_alpha_signs` symbol, then the per-plane
    /// `cfl_alpha_u`/`cfl_alpha_v` magnitudes (`|alpha| - 1`) for the non-zero planes, each under the
    /// sign-derived context.
    fn emit_cfl_alphas(&mut self, au: i32, av: i32) {
        let sign = |a: i32| -> usize {
            if a == 0 {
                0 // CFL_SIGN_ZERO
            } else if a < 0 {
                1 // CFL_SIGN_NEG
            } else {
                2 // CFL_SIGN_POS
            }
        };
        let (su, sv) = (sign(au), sign(av));
        // signs + 1 = 3 * signU + signV  ⇒  cfl_alpha_signs = 3 * signU + signV - 1.
        let signs = 3 * su + sv - 1;
        self.sym.encode_symbol(signs, &cdf::CFL_SIGN);
        if su != 0 {
            let ctx = (su - 1) * 3 + sv;
            self.sym
                .encode_symbol((au.abs() - 1) as usize, &cdf::CFL_ALPHA[ctx]);
        }
        if sv != 0 {
            let ctx = (sv - 1) * 3 + su;
            self.sym
                .encode_symbol((av.abs() - 1) as usize, &cdf::CFL_ALPHA[ctx]);
        }
    }

    #[allow(clippy::too_many_lines)]
    #[allow(
        clippy::too_many_lines,
        clippy::too_many_arguments,
        clippy::type_complexity
    )]
    fn code_coeffs(
        &mut self,
        plane: usize,
        x4: usize,
        y4: usize,
        block_w: usize,
        tx_size: TxSize,
        quant: &[i32],
        tx_sym: usize,
        intra_dir: usize,
    ) {
        let ptype = usize::from(plane > 0);
        let qctx = self.qctx;
        let (w, h) = (tx_size.width(), tx_size.height());
        // OPTION A: a rectangular block is coded as the single transform that fills it, so block == tx.
        let block_h = if w == h { block_w } else { h };
        let (w4, h4) = (w / 4, h / 4); // MI cells the transform spans on each axis
        // A >32 dimension (TX_64X64) codes only its top-left 32-wide/high sub-block (§7.13): the scan,
        // area and coefficient contexts use the `code_w × code_h` region, while the CDF tables and
        // `txb_skip`/`set_ctx` span the full transform.
        let (code_w, code_h) = (w.min(32), h.min(32));
        let bwl = code_w.trailing_zeros() as usize;
        let area = code_w * code_h;
        // The rectangular transforms share their bounding square's coefficient CDFs (`txSzCtx` = that
        // square's ctx; the `coeff_br` Min(txSzCtx,3) cap is already baked into the 64 row). Only the
        // scan order and the eob class (by coefficient count) differ.
        let up_sq = w.max(h);

        let rect_scan;
        let scan: &[usize] = match (w, h) {
            (4, 4) => &cdf::DEFAULT_SCAN_4X4,
            (8, 8) => &cdf::DEFAULT_SCAN_8X8,
            (16, 16) => &cdf::DEFAULT_SCAN_16X16,
            (32, 32) | (64, 64) => &cdf::DEFAULT_SCAN_32X32, // both code a 32×32 region
            _ => {
                rect_scan = cdf::default_scan(code_w, code_h);
                &rect_scan
            }
        };

        // Size-specific CDF tables, selected by the bounding square (same `[qctx][...]` layout).
        let (txb_skip, eob_extra, base_eob, base, br, offset): (
            &[[[u16; 2]; 13]; 4],
            &[[[[u16; 2]; 9]; 2]; 4],
            &[[[[u16; 3]; 4]; 2]; 4],
            &[[[[u16; 4]; 42]; 2]; 4],
            &[[[[u16; 4]; 21]; 2]; 4],
            &[[u8; 5]; 5],
        ) = match up_sq {
            4 => (
                &cdf::TXB_SKIP,
                &cdf::EOB_EXTRA,
                &cdf::COEFF_BASE_EOB,
                &cdf::COEFF_BASE,
                &cdf::COEFF_BR,
                &cdf::COEFF_BASE_CTX_OFFSET_4X4,
            ),
            8 => (
                &cdf::TXB_SKIP_8X8,
                &cdf::EOB_EXTRA_8X8,
                &cdf::COEFF_BASE_EOB_8X8,
                &cdf::COEFF_BASE_8X8,
                &cdf::COEFF_BR_8X8,
                &cdf::COEFF_BASE_CTX_OFFSET_8X8,
            ),
            16 => (
                &cdf::TXB_SKIP_16X16,
                &cdf::EOB_EXTRA_16X16,
                &cdf::COEFF_BASE_EOB_16X16,
                &cdf::COEFF_BASE_16X16,
                &cdf::COEFF_BR_16X16,
                &cdf::COEFF_BASE_CTX_OFFSET_8X8,
            ),
            32 => (
                &cdf::TXB_SKIP_32X32,
                &cdf::EOB_EXTRA_32X32,
                &cdf::COEFF_BASE_EOB_32X32,
                &cdf::COEFF_BASE_32X32,
                &cdf::COEFF_BR_32X32,
                &cdf::COEFF_BASE_CTX_OFFSET_8X8,
            ),
            // 64: `txSzCtx = 4` skip/base CDFs but `coeff_br` capped at TX_32X32 (Min(txSzCtx,3)).
            _ => (
                &cdf::TXB_SKIP_64X64,
                &cdf::EOB_EXTRA_64X64,
                &cdf::COEFF_BASE_EOB_64X64,
                &cdf::COEFF_BASE_64X64,
                &cdf::COEFF_BR_32X32,
                &cdf::COEFF_BASE_CTX_OFFSET_8X8,
            ),
        };

        // A rectangular transform uses an aspect-specific `coeff_base` offset table (§8.3.2): the
        // square table only fits `w == h`; wide and tall blocks differ at the low frequencies.
        let offset: &[[u8; 5]; 5] = if w > h {
            &cdf::COEFF_BASE_CTX_OFFSET_WIDE
        } else if w < h {
            &cdf::COEFF_BASE_CTX_OFFSET_TALL
        } else {
            offset
        };

        let mut eob = 0usize;
        for c in 0..area {
            if quant[scan[c]] != 0 {
                eob = c + 1;
            }
        }

        if w != h && plane == 0 && std::env::var("GAMUT_DBG").is_ok() {
            eprintln!(
                "ENC rect {}x{} eob={} scan5={:?}",
                w,
                h,
                eob,
                &scan[..5.min(scan.len())]
            );
        }
        let txb_ctx = self.txb_skip_ctx(plane, x4, y4, block_w, block_h, w, h);
        self.sym
            .encode_symbol(usize::from(eob == 0), &txb_skip[qctx][txb_ctx]);
        if eob == 0 {
            self.set_ctx(plane, x4, y4, w4, h4, 0, 0);
            return;
        }

        // transform_type (§5.11.39): signaled only for luma when not all-zero and base_q_idx > 0.
        // Reduced tx set + 4×4/8×8/16×16 intra ⇒ TX_SET_INTRA_2; `tx_sym` indexes
        // Tx_Type_Intra_Inv_Set2 = {IDTX, DCT_DCT, ADST_ADST, ADST_DCT, DCT_ADST}. The CDF is uniform
        // across intra direction for 4×4/8×8 but per-`intraDir` for 16×16 (§9.4). A 32×32 intra block
        // is TX_SET_DCTONLY (§5.11.48 `txSzSqrUp == TX_32X32`), so no transform type is signaled.
        // The set is TX_SET_INTRA_2 when `txSzSqrUp ≤ TX_16X16` (`up_sq ≤ 16`), else DCTONLY (no
        // type). The CDF is keyed by `txSzSqr = Min(w, h)`: uniform for ≤ 8, per-`intraDir` for 16×16.
        if self.qindex > 0 && plane == 0 && up_sq <= 16 {
            let tx_cdf: &[u16] = if w.min(h) == 16 {
                &cdf::INTRA_TX_TYPE_SET2_16X16[intra_dir]
            } else {
                &cdf::INTRA_TX_TYPE_SET2
            };
            self.sym.encode_symbol(tx_sym, tx_cdf);
        }

        // eob position (TX_CLASS_2D ⇒ eob_pt context 0). The eob class is the coded coefficient count
        // (`area`): 16/64/128/256/512/1024. The 512 and 1024 tables have no neighbour-context dimension.
        let eobpt = eobpt_from_eob(eob);
        match area {
            16 => self
                .sym
                .encode_symbol(eobpt - 1, &cdf::EOB_PT_16[qctx][ptype][0]),
            64 => self
                .sym
                .encode_symbol(eobpt - 1, &cdf::EOB_PT_64[qctx][ptype][0]),
            128 => self
                .sym
                .encode_symbol(eobpt - 1, &cdf::EOB_PT_128[qctx][ptype][0]),
            256 => self
                .sym
                .encode_symbol(eobpt - 1, &cdf::EOB_PT_256[qctx][ptype][0]),
            512 => self
                .sym
                .encode_symbol(eobpt - 1, &cdf::EOB_PT_512[qctx][ptype]),
            _ => self
                .sym
                .encode_symbol(eobpt - 1, &cdf::EOB_PT_1024[qctx][ptype]),
        }
        if eobpt >= 3 {
            let nbits = eobpt - 2;
            let base_eob_val = (1usize << (eobpt - 2)) + 1;
            let extra = eob - base_eob_val;
            self.sym.encode_symbol(
                (extra >> (nbits - 1)) & 1,
                &eob_extra[qctx][ptype][eobpt - 3],
            );
            let mut i = nbits as isize - 2;
            while i >= 0 {
                self.sym.encode_literal(((extra >> i) & 1) as u32, 1);
                i -= 1;
            }
        }

        // Base levels + base range, scanned from the last coefficient back to DC.
        let mut levels = vec![0i32; area];
        for c in (0..eob).rev() {
            let pos = scan[c];
            let level = quant[pos].abs();
            if c == eob - 1 {
                let ctx = coeff_base_eob_ctx(c, area);
                self.sym
                    .encode_symbol((level.min(3) - 1) as usize, &base_eob[qctx][ptype][ctx]);
            } else {
                let ctx = coeff_base_ctx(pos, &levels, bwl, code_w, code_h, offset);
                self.sym
                    .encode_symbol(level.min(3) as usize, &base[qctx][ptype][ctx]);
            }
            if level > NUM_BASE_LEVELS {
                let br_ctx = coeff_br_ctx(pos, &levels, bwl, code_w, code_h);
                let mut rem = level - 3;
                for _ in 0..4 {
                    let brv = rem.min(3);
                    self.sym
                        .encode_symbol(brv as usize, &br[qctx][ptype][br_ctx]);
                    rem -= brv;
                    if brv < 3 {
                        break;
                    }
                }
            }
            levels[pos] = level;
        }

        // Signs (DC sign is CDF-coded; the rest are raw bits) and golomb tails.
        for (c, &pos) in scan.iter().enumerate().take(eob) {
            let level = quant[pos].abs();
            if level != 0 {
                let neg = quant[pos] < 0;
                if c == 0 {
                    let ctx = self.dc_sign_ctx(plane, x4, y4, w4, h4);
                    self.sym
                        .encode_symbol(usize::from(neg), &cdf::DC_SIGN[ptype][ctx]);
                } else {
                    self.sym.encode_literal(u32::from(neg), 1);
                }
                if level > COEFF_BASE_PLUS_RANGE {
                    golomb(&mut self.sym, (level - COEFF_BASE_PLUS_RANGE) as u32);
                }
            }
        }

        let cul = levels.iter().sum::<i32>().min(63) as u8;
        let dc_cat = if quant[0] == 0 {
            0
        } else if quant[0] < 0 {
            1
        } else {
            2
        };
        self.set_ctx(plane, x4, y4, w4, h4, cul, dc_cat);
    }

    /// Writes `culLevel`/`dcCategory` into the above/left level-context arrays for every MI cell the
    /// transform block spans (§5.11.39: `for i in 0..w4`/`0..h4`). `n4 = Tx_Width / 4`.
    #[allow(clippy::too_many_arguments)]
    fn set_ctx(
        &mut self,
        plane: usize,
        x4: usize,
        y4: usize,
        w4: usize,
        h4: usize,
        cul: u8,
        dc: u8,
    ) {
        for k in 0..w4 {
            if x4 + k < self.mi_cols {
                self.above_level[plane][x4 + k] = cul;
                self.above_dc[plane][x4 + k] = dc;
            }
        }
        for k in 0..h4 {
            if y4 + k < self.mi_rows {
                self.left_level[plane][y4 + k] = cul;
                self.left_dc[plane][y4 + k] = dc;
            }
        }
    }

    /// `all_zero` (txb_skip) context (§8.3.2). `block_w` is the residual-block width, `tx_w` the
    /// transform width: luma uses ctx 0 when the transform covers the whole block (the only case the
    /// lossy path reaches, since `TX_MODE_LARGEST` ⇒ tx == block); otherwise (lossless, where a
    /// larger block is split into 4×4 transforms) it falls back to the neighbour-level classification.
    #[allow(clippy::too_many_arguments)]
    fn txb_skip_ctx(
        &self,
        plane: usize,
        x4: usize,
        y4: usize,
        block_w: usize,
        block_h: usize,
        tx_w: usize,
        tx_h: usize,
    ) -> usize {
        let (w4, h4) = (tx_w / 4, tx_h / 4);
        if plane == 0 {
            if block_w == tx_w && block_h == tx_h {
                return 0;
            }
            let mut top = 0i32;
            let mut left = 0i32;
            for k in 0..w4 {
                if x4 + k < self.mi_cols {
                    top = top.max(i32::from(self.above_level[0][x4 + k]));
                }
            }
            for k in 0..h4 {
                if y4 + k < self.mi_rows {
                    left = left.max(i32::from(self.left_level[0][y4 + k]));
                }
            }
            if top == 0 && left == 0 {
                1
            } else if top == 0 || left == 0 {
                2 + usize::from(top.max(left) > 3)
            } else if top.max(left) <= 3 {
                4
            } else if top.min(left) <= 3 {
                5
            } else {
                6
            }
        } else {
            let mut above = 0u8;
            let mut left = 0u8;
            for k in 0..w4 {
                if x4 + k < self.mi_cols {
                    above |= self.above_level[plane][x4 + k] | self.above_dc[plane][x4 + k];
                }
            }
            for k in 0..h4 {
                if y4 + k < self.mi_rows {
                    left |= self.left_level[plane][y4 + k] | self.left_dc[plane][y4 + k];
                }
            }
            let mut ctx = usize::from(above != 0) + usize::from(left != 0) + 7;
            // bw*bh > tw*th ⇒ the block is larger than the transform (4×4 tx in an ≥8×8 lossless block,
            // or a rectangular block whose chroma is split). Under TX_MODE_LARGEST tx == block, so this
            // never adds for the square lossy path.
            if block_w * block_h > tx_w * tx_h {
                ctx += 3;
            }
            ctx
        }
    }

    /// `dc_sign` context (§8.3.2): the sum of DC-sign contributions (`+1` positive, `-1` negative)
    /// over every MI cell the transform spans (`n4 = Tx_Width / 4` above cells and `n4` left cells).
    /// A single cell each is correct only for `TX_4X4`; an 8×8 transform with non-uniform neighbour
    /// DC categories needs the full sum or the arithmetic decode diverges.
    fn dc_sign_ctx(&self, plane: usize, x4: usize, y4: usize, w4: usize, h4: usize) -> usize {
        let mut s = 0i32;
        for k in 0..w4 {
            if x4 + k < self.mi_cols {
                match self.above_dc[plane][x4 + k] {
                    1 => s -= 1,
                    2 => s += 1,
                    _ => {}
                }
            }
        }
        for k in 0..h4 {
            if y4 + k < self.mi_rows {
                match self.left_dc[plane][y4 + k] {
                    1 => s -= 1,
                    2 => s += 1,
                    _ => {}
                }
            }
        }
        if s < 0 {
            1
        } else if s > 0 {
            2
        } else {
            0
        }
    }
}

/// Selects the partition CDF by `bsl` (`Mi_Width_Log2`); M0 never uses 128×128 superblocks.
fn partition_cdf(bsl: usize, ctx: usize) -> &'static [u16] {
    match bsl {
        1 => &cdf::PARTITION_W8[ctx],
        2 => &cdf::PARTITION_W16[ctx],
        3 => &cdf::PARTITION_W32[ctx],
        _ => &cdf::PARTITION_W64[ctx],
    }
}

/// Derives the 2-symbol `split_or_horz` CDF from the partition CDF (§8.3.2): the vertical-ish
/// partition probabilities are folded into the "split" outcome.
fn split_or_horz_cdf(p: &[u16]) -> [u16; 2] {
    let psum = (p[2] - p[1])
        + (p[3] - p[2])
        + (p[4] - p[3])
        + (p[6] - p[5])
        + (p[7] - p[6])
        + (p[9] - p[8]);
    [32768 - psum, 32768]
}

/// Derives the 2-symbol `split_or_vert` CDF from the partition CDF (§8.3.2).
fn split_or_vert_cdf(p: &[u16]) -> [u16; 2] {
    let psum = (p[1] - p[0])
        + (p[3] - p[2])
        + (p[4] - p[3])
        + (p[5] - p[4])
        + (p[6] - p[5])
        + (p[8] - p[7]);
    [32768 - psum, 32768]
}

/// `eobPt` from `eob` (inverts `eob = (eobPt < 2) ? eobPt : (1 << (eobPt-2)) + 1`, §5.11.39).
fn eobpt_from_eob(eob: usize) -> usize {
    if eob <= 1 {
        eob
    } else {
        (32 - ((eob - 1) as u32).leading_zeros()) as usize + 1
    }
}

/// `get_coeff_base_ctx` for the end-of-block coefficient (§8.3.2), `isEob == 1`, for a square
/// `TX_CLASS_2D` block of `area` (`= width * height`) coefficients. Maps the scan index `c` to one
/// of the 4 `coeff_base_eob` contexts.
fn coeff_base_eob_ctx(c: usize, area: usize) -> usize {
    if c == 0 {
        0
    } else if c <= area / 8 {
        1
    } else if c <= area / 4 {
        2
    } else {
        3
    }
}

/// `get_coeff_base_ctx` for a non-EOB coefficient (§8.3.2) of a square `TX_CLASS_2D` block: `bwl` is
/// `Tx_Width_Log2`, `n = 1 << bwl` is the (square) transform side, and `offset` is the
/// `Coeff_Base_Ctx_Offset[txSz]` table. `levels` holds the magnitudes already decoded along the
/// reverse scan, indexed by the linear coefficient position.
fn coeff_base_ctx(
    pos: usize,
    levels: &[i32],
    bwl: usize,
    w: usize,
    h: usize,
    offset: &[[u8; 5]; 5],
) -> usize {
    let (row, col) = (pos >> bwl, pos & (w - 1));
    let mut mag = 0i32;
    for &(dr, dc) in &cdf::SIG_REF_DIFF_OFFSET_2D {
        let (rr, cc) = (row + dr, col + dc);
        if rr < h && cc < w {
            mag += levels[(rr << bwl) + cc].abs().min(3);
        }
    }
    let ctx = (((mag + 1) >> 1).min(4)) as usize;
    if row == 0 && col == 0 {
        return 0;
    }
    ctx + usize::from(offset[row.min(4)][col.min(4)])
}

/// `get_coeff_br_ctx` for a `TX_CLASS_2D` block (§8.3.2): `bwl = Tx_Width_Log2`; the coded region is
/// `w × h` (`w == h` for a square transform, the top-left `32 × 32` for `TX_64X64`).
fn coeff_br_ctx(pos: usize, levels: &[i32], bwl: usize, w: usize, h: usize) -> usize {
    let (row, col) = (pos >> bwl, pos & (w - 1));
    let mut mag = 0i32;
    for &(dr, dc) in &cdf::MAG_REF_OFFSET_2D {
        let (rr, cc) = (row + dr, col + dc);
        if rr < h && cc < w {
            mag += levels[(rr << bwl) + cc].abs().min(15);
        }
    }
    let mag = (((mag + 1) >> 1).min(6)) as usize;
    if pos == 0 {
        mag
    } else if row < 2 && col < 2 {
        mag + 7
    } else {
        mag + 14
    }
}

/// `NS(n)` non-symmetric literal (§4.10.7): codes a value in `0..n` with `floor(log2(n))` or +1 bits.
fn encode_ns(sym: &mut SymbolEncoder, v: u32, n: u32) {
    let w = (32 - n.leading_zeros()) as i32; // FloorLog2(n) + 1
    let m = (1u32 << w) - n;
    if v < m {
        sym.encode_literal(v, (w - 1) as u32);
    } else {
        let x = v + m;
        sym.encode_literal(x >> 1, (w - 1) as u32);
        sym.encode_literal(x & 1, 1);
    }
}

/// `encode_subexp(v)` — the inverse of §4.10.8 `decode_subexp`: codes `v` in `0..num_syms` with a
/// sub-exponential prefix keyed by `k`.
fn encode_subexp(sym: &mut SymbolEncoder, v: u32, num_syms: u32, k: u32) {
    let (mut i, mut mk) = (0u32, 0u32);
    loop {
        let b2 = if i != 0 { k + i - 1 } else { k };
        let a = 1u32 << b2;
        if num_syms <= mk + 3 * a {
            encode_ns(sym, v - mk, num_syms - mk);
            return;
        } else if v >= mk + a {
            sym.encode_literal(1, 1); // subexp_more_bits
            i += 1;
            mk += a;
        } else {
            sym.encode_literal(0, 1);
            sym.encode_literal(v - mk, b2);
            return;
        }
    }
}

/// Recenters `v` around reference `r` (the inverse of §4.10.10 `inverse_recenter`).
fn recenter(r: i32, v: i32) -> u32 {
    if v > 2 * r {
        v as u32
    } else if v < r {
        (2 * (r - v) - 1) as u32
    } else {
        (2 * (v - r)) as u32
    }
}

/// `encode_unsigned_subexp_with_ref(v, mx, k, r)` (§4.10.9 inverse): codes `v` in `0..mx` against
/// reference `r` so that coding `v == r` emits the shortest (zero-delta) code.
fn encode_subexp_with_ref(sym: &mut SymbolEncoder, v: i32, mx: i32, k: u32, r: i32) {
    let recentered = if (r << 1) <= mx {
        recenter(r, v)
    } else {
        recenter(mx - 1 - r, mx - 1 - v)
    };
    encode_subexp(sym, recentered, mx as u32, k);
}

/// Exp-Golomb tail used for coefficient magnitudes above the base-range cap (§5.11.39).
fn golomb(sym: &mut SymbolEncoder, x: u32) {
    let len = 32 - x.leading_zeros(); // bit length, x >= 1
    for _ in 0..(len - 1) {
        sym.encode_literal(0, 1);
    }
    sym.encode_literal(1, 1);
    let mut i = len as isize - 2;
    while i >= 0 {
        sym.encode_literal((x >> i) & 1, 1);
        i -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamut_color::Planar8;

    /// A 4×4 mid-grey image is enough to construct an encoder; selection depends only on the
    /// residual and the quantizers, not the plane contents.
    fn grey4x4() -> Planar8 {
        Planar8::from_rgb8_identity(&[128u8; 4 * 4 * 3], 4, 4).unwrap()
    }

    #[test]
    fn block_exceeds_frame_flags_only_overhang() {
        // Fully inside, with room to spare.
        assert!(!block_exceeds_frame(0, 0, 8, 16, 16));
        // Exactly reaching each edge fits (pins `>` vs `>=`, and `+` vs `*` at non-zero positions).
        assert!(!block_exceeds_frame(0, 0, 8, 8, 8)); // r+8 == 8 and c+8 == 8
        assert!(!block_exceeds_frame(2, 0, 6, 8, 8)); // r+6 == 8 exactly
        assert!(!block_exceeds_frame(0, 2, 6, 8, 8)); // c+6 == 8 exactly
        // Overhanging only the bottom edge, or only the right edge, must still flag (pins `||` vs
        // `&&`, the row/col `>` mutants, and the `c*num4x4`/`r*num4x4` mutants at position 0).
        assert!(block_exceeds_frame(0, 0, 8, 6, 99)); // rows overhang only
        assert!(block_exceeds_frame(0, 0, 8, 99, 6)); // cols overhang only
    }

    #[test]
    fn tx_type_selection_matches_set2_mapping() {
        let p = grey4x4();
        let e = FrameEncoder::new(&p, 48);

        // A flat residual concentrates into the DCT DC bin (one coefficient), so DCT_DCT wins and
        // is signaled as Tx_Type_Intra_Inv_Set2 symbol 1.
        let (tx, sym, _) = e.select_tx_type(&[40i32; 16], TxSize::Tx4x4);
        assert!(matches!(tx, TxType::DctDct));
        assert_eq!(sym, 1);

        // A single non-DC impulse stays a single coefficient under the identity transform but
        // spreads across many DCT bins, so IDTX wins and is signaled as symbol 0.
        let mut impulse = [0i32; 16];
        impulse[5] = 220;
        let (tx, sym, levels) = e.select_tx_type(&impulse, TxSize::Tx4x4);
        assert!(matches!(tx, TxType::Idtx));
        assert_eq!(sym, 0);
        assert_eq!(levels.iter().filter(|&&l| l != 0).count(), 1);
    }

    #[test]
    fn tx_type_selection_is_deterministic() {
        let p = grey4x4();
        let e = FrameEncoder::new(&p, 90);
        let mut res = [0i32; 16];
        for (i, v) in res.iter_mut().enumerate() {
            *v = (i as i32 - 8) * 17;
        }
        let a = e.select_tx_type(&res, TxSize::Tx4x4);
        let b = e.select_tx_type(&res, TxSize::Tx4x4);
        assert_eq!(a.1, b.1);
        assert_eq!(a.2, b.2);
    }

    #[test]
    fn predictors_match_spec_formulas() {
        // 12×12 mid-grey image → the coded grid has a block at (4,4) with available neighbours.
        let p = Planar8::from_rgb8_identity(&[128u8; 12 * 12 * 3], 12, 12).unwrap();
        let mut e = FrameEncoder::new(&p, 32);
        let cw = e.coded_w;
        // Above row at y=3, x=4..8; left col at x=3, y=4..8; top-left at (3,3).
        for j in 0..4 {
            e.recon[0][3 * cw + (4 + j)] = 100;
        }
        for i in 0..4 {
            e.recon[0][(4 + i) * cw + 3] = 50;
        }
        e.recon[0][3 * cw + 3] = 80;

        // PAETH: base = 100 + 50 - 80 = 70; |70-80| (top-left) is smallest, so every sample = 80.
        let paeth = e.predict_4x4(0, 4, 4, PAETH_PRED);
        assert_eq!(paeth, [80; 16]);

        // SMOOTH_V row 0 uses weight 255: (255*100 + 1*50 + 128) >> 8 = 100; row 3 weight 64:
        // (64*100 + 192*50 + 128) >> 8 = 63.
        let sv = e.predict_4x4(0, 4, 4, SMOOTH_V_PRED);
        assert_eq!(sv[0], 100);
        assert_eq!(sv[12], 63);

        // SMOOTH_H column 0 uses weight 255: (255*50 + 1*100 + 128) >> 8 = 50.
        let sh = e.predict_4x4(0, 4, 4, SMOOTH_H_PRED);
        assert_eq!(sh[0], 50);

        // SMOOTH (i=0,j=0): (255*100 + 1*50 + 255*50 + 1*100 + 256) >> 9 = 75.
        let sm = e.predict_4x4(0, 4, 4, SMOOTH_PRED);
        assert_eq!(sm[0], 75);
    }

    #[test]
    fn general_directional_matches_4x4_path() {
        // The size-generic directional predictor (used for 8×8) must reproduce the validated 4×4
        // predictor exactly when n = 4 and angle_delta = 0, for every directional mode — including
        // the zone-1/3 modes that read the above-right / below-left extension. All BlockDecoded
        // cells are marked available so both code paths read the same extended references.
        let p = Planar8::from_rgb8_identity(&[128u8; 16 * 16 * 3], 16, 16).unwrap();
        let mut e = FrameEncoder::new(&p, 32);
        let cw = e.coded_w;
        for y in 0..e.coded_h {
            for x in 0..cw {
                e.recon[0][y * cw + x] = ((x * 7 + y * 13 + 5) & 0xff) as u16;
            }
        }
        e.block_decoded.iter_mut().for_each(|b| *b = 1);
        for &mode in &DIRECTIONAL_MODES {
            let general = e.predict_directional(0, 4, 4, 4, mode, 0);
            let baseline = e.predict_4x4(0, 4, 4, mode).to_vec();
            assert_eq!(general, baseline, "directional mode {mode} (n=4) mismatch");
        }
    }

    #[test]
    fn directional_8x8_angle_delta_in_range() {
        // 8×8 directional prediction with every angle_delta must stay in [0, 255] (no OOB / overflow
        // across all four zones), exercising the extended 16-sample reference arrays.
        let p = Planar8::from_rgb8_identity(&[128u8; 32 * 32 * 3], 32, 32).unwrap();
        let mut e = FrameEncoder::new(&p, 32);
        let cw = e.coded_w;
        for y in 0..e.coded_h {
            for x in 0..cw {
                e.recon[0][y * cw + x] = (((x * 11) ^ (y * 5 + 9)) & 0xff) as u16;
            }
        }
        e.block_decoded.iter_mut().for_each(|b| *b = 1);
        for &mode in &DIRECTIONAL_MODES {
            for ad in -3..=3i8 {
                let pred = e.predict_directional(0, 8, 8, 8, mode, ad);
                assert_eq!(pred.len(), 64);
                assert!(
                    pred.iter().all(|&x| (0..=255).contains(&x)),
                    "8×8 mode {mode} angle_delta {ad} out of range"
                );
            }
        }
    }

    #[test]
    fn reference_sample_fallbacks_at_frame_corner() {
        // Block at (0,0): no above, no left. Defaults are AboveRow=127, LeftCol=129, top-left=128.
        let p = grey4x4();
        let e = FrameEncoder::new(&p, 32);
        // PAETH base = 127 + 129 - 128 = 128; distances are |128-129|=1 (left), |128-127|=1 (top),
        // |128-128|=0 (top-left), so the top-left default (128) wins and every sample is 128.
        assert_eq!(e.predict_4x4(0, 0, 0, PAETH_PRED), [128; 16]);
    }

    #[test]
    fn directional_predictors_match_spec() {
        // Block at (4,4) with a ramped above row [60,90,120,150], left column [40,80,120,160],
        // top-left 100.
        let p = Planar8::from_rgb8_identity(&[128u8; 12 * 12 * 3], 12, 12).unwrap();
        let mut e = FrameEncoder::new(&p, 32);
        let cw = e.coded_w;
        for (j, &v) in [60u16, 90, 120, 150].iter().enumerate() {
            e.recon[0][3 * cw + (4 + j)] = v;
        }
        for (i, &v) in [40u16, 80, 120, 160].iter().enumerate() {
            e.recon[0][(4 + i) * cw + 3] = v;
        }
        e.recon[0][3 * cw + 3] = 100;

        // V (pAngle 90): every row is a copy of the above row.
        let v = e.predict_4x4(0, 4, 4, V_PRED);
        assert_eq!(&v[0..4], &[60, 90, 120, 150]);
        assert_eq!(&v[12..16], &[60, 90, 120, 150]);
        // H (pAngle 180): every column is a copy of the left column.
        let h = e.predict_4x4(0, 4, 4, H_PRED);
        assert_eq!([h[0], h[4], h[8], h[12]], [40, 80, 120, 160]);
        // D135: the main diagonal carries the top-left corner; row 0 = [TL, above0, above1, above2].
        let d135 = e.predict_4x4(0, 4, 4, D135_PRED);
        assert_eq!(&d135[0..4], &[100, 60, 90, 120]);
        assert_eq!([d135[0], d135[5], d135[10], d135[15]], [100, 100, 100, 100]);
        // All directional outputs stay in range.
        for &m in &[D113_PRED, D157_PRED] {
            assert!(
                e.predict_4x4(0, 4, 4, m)
                    .iter()
                    .all(|&x| (0..=255).contains(&x))
            );
        }
    }

    #[test]
    fn filter_intra_predictor_matches_spec() {
        // Block at (4,4) with a single non-zero reference: AboveRow[0] = 16, every other reference
        // (AboveRow[1..3], LeftCol[0..3], top-left) = 0. FILTER_DC_PRED taps then give a hand-traced
        // recursive result (the lower 4×2 filters the upper 4×2's predicted row 1, §7.11.2.3).
        let p = Planar8::from_rgb8_identity(&[128u8; 12 * 12 * 3], 12, 12).unwrap();
        let mut e = FrameEncoder::new(&p, 32);
        let cw = e.coded_w;
        e.recon[0][3 * cw + 4] = 16; // AboveRow[0]
        let pred = e.predict_filter_intra(0, 4, 4, 4, 0);
        assert_eq!(
            pred,
            [10, 2, 1, 1, 6, 2, 2, 1, 4, 2, 2, 1, 2, 2, 2, 1],
            "FILTER_DC recursive prediction"
        );
    }

    #[test]
    fn filter_intra_preserves_flat_reference() {
        // Every Intra_Filter_Taps row sums to 16 = 1 << INTRA_FILTER_SCALE_BITS, so a constant
        // reference must reproduce that constant for all five filter-intra modes.
        let p = Planar8::from_rgb8_identity(&[128u8; 12 * 12 * 3], 12, 12).unwrap();
        let mut e = FrameEncoder::new(&p, 32);
        let cw = e.coded_w;
        for k in 0..8 {
            e.recon[0][3 * cw + (4 + k)] = 77; // above row + above-right
            e.recon[0][(4 + k) * cw + 3] = 77; // left col + below-left
        }
        e.recon[0][3 * cw + 3] = 77; // top-left
        for fi in 0..5u8 {
            assert_eq!(
                e.predict_filter_intra(0, 4, 4, 4, fi),
                [77; 16],
                "filter-intra mode {fi} should preserve a flat reference"
            );
        }
    }

    #[test]
    fn cfl_prediction_matches_spec() {
        // Reconstructed luma with a pure column gradient 80,88,96,104 (constant down each column).
        // For 4:4:4, L = luma*8, lumaAvg = Round2(ΣL, 4); with alpha = 2 the per-column high-freq
        // term Round2Signed(2*(L-avg), 6) is -3,-1,+1,+3, added to a flat DC chroma prediction.
        let p = Planar8::from_rgb8_identity(&[128u8; 12 * 12 * 3], 12, 12).unwrap();
        let mut e = FrameEncoder::new(&p, 32);
        let cw = e.coded_w;
        for i in 0..4 {
            for j in 0..4 {
                e.recon[0][(4 + i) * cw + (4 + j)] = (80 + 8 * j) as u16;
            }
        }
        let mut pred = [128i32; 16];
        e.apply_cfl(&mut pred, 4, 4, 2, 4);
        assert_eq!(
            pred,
            [
                125, 127, 129, 131, 125, 127, 129, 131, 125, 127, 129, 131, 125, 127, 129, 131
            ]
        );
        // alpha = 0 is a no-op (plain DC).
        let mut flat = [100i32; 16];
        e.apply_cfl(&mut flat, 4, 4, 0, 4);
        assert_eq!(flat, [100; 16]);
    }

    #[test]
    fn d45_reads_above_right_when_available() {
        // Block at (4,4) ⇒ superblock-relative (by, bx) = (1, 1). Mark the above-right 4×4 decoded
        // so D45 (pAngle 45) reads the real extended above-row samples 4..8.
        let p = Planar8::from_rgb8_identity(&[128u8; 12 * 12 * 3], 12, 12).unwrap();
        let mut e = FrameEncoder::new(&p, 32);
        let cw = e.coded_w;
        for (k, &v) in [10u16, 20, 30, 40, 50, 60, 70, 80].iter().enumerate() {
            e.recon[0][3 * cw + (4 + k)] = v; // above row incl. above-right (x=4..12)
        }
        e.clear_block_decoded(0, 0);
        e.block_decoded[BD_STRIDE + 3] = 1; // (by-1, bx+1) = (0, 2) decoded ⇒ haveAboveRight
        // dx = 64 ⇒ shift = 0, base = (i+1)+j; row 0 = above[1..5] = [20,30,40,50] (50 is an
        // above-right sample) and the bottom-right clamps to above[w+h-1] = above[7] = 80.
        let d45 = e.predict_4x4(0, 4, 4, D45_PRED);
        assert_eq!(&d45[0..4], &[20, 30, 40, 50]);
        assert_eq!(d45[15], 80);
    }
}
