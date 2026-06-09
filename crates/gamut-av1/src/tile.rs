//! The single-tile, all-intra encoder: superblock/partition iteration (§5.11.2/.4), intra
//! prediction (§7.11.2), the forward transform (via `gamut-dsp`), and coefficient coding with
//! full context derivation (§5.11.39, §8.3.2).
//!
//! Two paths share this code, selected by `qindex` (`base_q_idx`):
//! - **Lossless** (`qindex == 0`): forced `TX_4X4` Walsh–Hadamard; prediction neighbours are the
//!   source samples (which equal the reconstruction under lossless); partitions are
//!   `PARTITION_NONE` except at the right/bottom frame edges.
//! - **Lossy** (`qindex > 0`): `TX_4X4` with quantization. Blocks are forced all-4×4
//!   (`PARTITION_SPLIT` everywhere) so each uses a 4×4 transform under `TX_MODE_LARGEST` with no
//!   tx-depth signaling. The luma prediction mode is chosen per block from all 13 intra modes
//!   (`DC`, the eight directional `V`/`H`/`D45`/`D135`/`D113`/`D157`/`D203`/`D67`, and
//!   `SMOOTH`/`SMOOTH_V`/`SMOOTH_H`/`PAETH`, §7.11.2) plus recursive **filter-intra** (§7.11.2.3,
//!   signaled as `DC_PRED` + `use_filter_intra`); it is signaled with the above/left mode context.
//!   The luma transform type is chosen from `TX_SET_INTRA_2`
//!   (`{IDTX, DCT_DCT, ADST_ADST, ADST_DCT, DCT_ADST}`) and signaled; chroma is `DC_PRED` or
//!   **chroma-from-luma** (`UV_CFL_PRED`, §7.11.5, when it beats DC) + `DCT_DCT`. Prediction reads a
//!   **reconstruction buffer** that the encoder maintains exactly as
//!   the decoder would (predict → transform → quantize → dequantize → inverse-transform → add →
//!   store), so the encoder's reconstruction is bit-exact with a conformant decoder's output.
//!
//! The frame is coded on the MI-unit grid (`mi_cols*4 × mi_rows*4`, dimensions rounded up to a
//! multiple of 8); the out-of-frame padding is edge-replicated and cropped away on decode.

use crate::cdf;
use crate::quant::{ac_q, dc_q, dequant, quantize};
use crate::transform::{TxSize, TxType, forward_transform_2d, inverse_transform_2d};
use gamut_bitstream::SymbolEncoder;
use gamut_color::Planar8;

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

/// How a single transform block's samples are predicted. `mode` is the intra mode (`DC_PRED` for
/// chroma); `filter_intra` overrides luma prediction with a recursive filter-intra mode (§7.11.2.3);
/// `cfl_alpha` adds the chroma-from-luma high-frequency term (§7.11.5). At most one of the latter
/// two is set, and only on its respective plane.
#[derive(Clone, Copy)]
struct Pred {
    mode: u8,
    filter_intra: Option<u8>,
    cfl_alpha: Option<i32>,
}

/// The reconstructed (decoded) planes on the coded MI grid, used as prediction neighbours in the
/// lossy path and exported for the bit-exact decoder cross-check.
pub(crate) struct Reconstruction {
    pub planes: [Vec<u8>; 3],
    pub coded_w: usize,
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
    /// `base_q_idx`; 0 ⇒ lossless WHT path, > 0 ⇒ lossy DCT path.
    qindex: u8,
    /// Coefficient-CDF quantizer context (§8.3.2): 0 if `qindex ≤ 20`, 1 if ≤ 60, 2 if ≤ 120, else 3.
    qctx: usize,
    dc_quant: i32,
    ac_quant: i32,
    /// Reconstructed samples per plane on the coded grid (lossy path only).
    recon: [Vec<u8>; 3],
    sym: SymbolEncoder,
    above_level: [Vec<u8>; 3],
    above_dc: [Vec<u8>; 3],
    left_level: [Vec<u8>; 3],
    left_dc: [Vec<u8>; 3],
    /// `Mi_Width_Log2` of the block covering each MI cell (for the partition context).
    mi_bsl: Vec<u8>,
    /// The luma intra mode (`YMode`) chosen for each MI cell, for the `intra_frame_y_mode` context.
    mi_ymode: Vec<u8>,
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
        let recon = if qindex > 0 {
            [
                vec![0u8; coded_w * coded_h],
                vec![0u8; coded_w * coded_h],
                vec![0u8; coded_w * coded_h],
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
            qindex,
            qctx: match qindex {
                0..=20 => 0,
                21..=60 => 1,
                61..=120 => 2,
                _ => 3,
            },
            dc_quant: dc_q(8, i32::from(qindex)),
            ac_quant: ac_q(8, i32::from(qindex)),
            recon,
            sym: SymbolEncoder::new(),
            above_level: [vec![0; mi_cols], vec![0; mi_cols], vec![0; mi_cols]],
            above_dc: [vec![0; mi_cols], vec![0; mi_cols], vec![0; mi_cols]],
            left_level: [vec![0; mi_rows], vec![0; mi_rows], vec![0; mi_rows]],
            left_dc: [vec![0; mi_rows], vec![0; mi_rows], vec![0; mi_rows]],
            mi_bsl: vec![0; mi_cols * mi_rows],
            mi_ymode: vec![0; mi_cols * mi_rows],
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
        let sb_width4 = (self.mi_cols - c) as isize;
        let sb_height4 = (self.mi_rows - r) as isize;
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
    pub(crate) fn encode(mut self) -> (Vec<u8>, Reconstruction) {
        const SB4: usize = 16; // 64×64 superblock in MI units
        let mut r = 0;
        while r < self.mi_rows {
            for plane in 0..3 {
                self.left_level[plane].iter_mut().for_each(|v| *v = 0);
                self.left_dc[plane].iter_mut().for_each(|v| *v = 0);
            }
            let mut c = 0;
            while c < self.mi_cols {
                self.sb_r = r;
                self.sb_c = c;
                self.clear_block_decoded(r, c);
                self.encode_partition(r, c, 64);
                c += SB4;
            }
            r += SB4;
        }
        // Deblocking loop filter (§7.14): a post-process on the final reconstruction (intra
        // prediction during encoding read the pre-filter samples, so this does not affect any
        // prediction). The lossless path is `CodedLossless`, where the filter is disabled.
        let mut planes = self.recon;
        if self.qindex > 0 {
            crate::filter::deblock(
                &mut planes,
                self.coded_w,
                self.width,
                self.height,
                self.qindex,
            );
            // CDEF reads the deblocked reconstruction and produces a deringed one (§7.15).
            planes = crate::filter::cdef(&planes, self.coded_w, self.qindex);
        }
        let recon = Reconstruction {
            planes,
            coded_w: self.coded_w,
        };
        (self.sym.finish(), recon)
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

        let split = if bw < 8 {
            false // PARTITION_NONE forced, no symbol
        } else if has_rows && has_cols {
            let ctx = self.partition_ctx(r, c, bsl);
            if self.qindex > 0 {
                // Lossy: force PARTITION_SPLIT down to 4×4 so every block uses a 4×4 transform.
                self.sym.encode_symbol(3, partition_cdf(bsl, ctx));
                true
            } else {
                self.sym.encode_symbol(0, partition_cdf(bsl, ctx)); // PARTITION_NONE
                false
            }
        } else if has_cols {
            let ctx = self.partition_ctx(r, c, bsl);
            let cdf2 = split_or_horz_cdf(partition_cdf(bsl, ctx));
            self.sym.encode_symbol(1, &cdf2); // split
            true
        } else if has_rows {
            let ctx = self.partition_ctx(r, c, bsl);
            let cdf2 = split_or_vert_cdf(partition_cdf(bsl, ctx));
            self.sym.encode_symbol(1, &cdf2); // split
            true
        } else {
            true // forced PARTITION_SPLIT, no symbol
        };

        if !split {
            self.encode_block(r, c, bw);
        } else {
            let h = bw / 2;
            self.encode_partition(r, c, h);
            self.encode_partition(r, c + half, h);
            self.encode_partition(r + half, c, h);
            self.encode_partition(r + half, c + half, h);
        }
    }

    fn partition_ctx(&self, r: usize, c: usize, bsl: usize) -> usize {
        let above = r > 0 && usize::from(self.mi_bsl[(r - 1) * self.mi_cols + c]) < bsl;
        let left = c > 0 && usize::from(self.mi_bsl[r * self.mi_cols + (c - 1)]) < bsl;
        usize::from(left) * 2 + usize::from(above)
    }

    fn encode_block(&mut self, r: usize, c: usize, bw: usize) {
        let n4 = bw / 4;
        let bsl = n4.trailing_zeros() as u8;

        // intra_frame_mode_info: skip=0 (ctx 0), then y_mode and uv_mode. The lossless path always
        // predicts DC_PRED; the lossy path searches the non-directional luma modes per 4×4 block
        // (blocks are all 4×4 there) and signals the choice with the above/left mode context.
        self.sym.encode_symbol(0, &cdf::SKIP[0]);
        let (y_mode, filter_intra) = if self.qindex > 0 {
            self.select_luma_mode(c * 4, r * 4)
        } else {
            (DC_PRED, None)
        };
        let amode = cdf::INTRA_MODE_CONTEXT[if r > 0 {
            usize::from(self.mi_ymode[(r - 1) * self.mi_cols + c])
        } else {
            usize::from(DC_PRED)
        }];
        let lmode = cdf::INTRA_MODE_CONTEXT[if c > 0 {
            usize::from(self.mi_ymode[r * self.mi_cols + (c - 1)])
        } else {
            usize::from(DC_PRED)
        }];
        self.sym
            .encode_symbol(usize::from(y_mode), &cdf::INTRA_FRAME_Y_MODE[amode][lmode]);
        // uv_mode. Its CDF is indexed by the luma mode. CfL is allowed when Max(w,h) <= 32 (always
        // for the lossy 4×4 blocks, where bw == 4); the encoder picks chroma-from-luma when it beats
        // plain DC_PRED, then emits read_cfl_alphas. Otherwise uv_mode = DC_PRED. The decoder reads
        // this right after intra_frame_y_mode, so it must match the signaled luma mode.
        let cfl = if self.qindex > 0 && bw == 4 {
            self.select_cfl(c * 4, r * 4)
        } else {
            None
        };
        let ym = usize::from(y_mode);
        if bw == 4 {
            let uv = if cfl.is_some() { UV_CFL_PRED } else { DC_PRED };
            self.sym
                .encode_symbol(usize::from(uv), &cdf::UV_MODE_CFL_ALLOWED[ym]);
            if let Some((au, av)) = cfl {
                self.emit_cfl_alphas(au, av);
            }
        } else {
            self.sym.encode_symbol(0, &cdf::UV_MODE_CFL_NOT_ALLOWED[ym]);
        }

        // filter_intra_mode_info (§5.11.24): with enable_filter_intra = 1 (lossy path), every
        // DC_PRED luma block ≤ 32px signals use_filter_intra (PaletteSizeY is always 0 here). The
        // 4×4 blocks always satisfy the size bound, so the CDF is the BLOCK_4X4 row.
        if self.qindex > 0 && y_mode == DC_PRED {
            self.sym
                .encode_symbol(usize::from(filter_intra.is_some()), &cdf::FILTER_INTRA_4X4);
            if let Some(fi) = filter_intra {
                self.sym
                    .encode_symbol(usize::from(fi), &cdf::FILTER_INTRA_MODE);
            }
        }

        for y in 0..n4 {
            for x in 0..n4 {
                let (rr, cc) = (r + y, c + x);
                if rr < self.mi_rows && cc < self.mi_cols {
                    self.mi_bsl[rr * self.mi_cols + cc] = bsl;
                    self.mi_ymode[rr * self.mi_cols + cc] = y_mode;
                }
            }
        }

        // residual(): per plane, raster of 4×4 transform blocks (Lossless ⇒ TX_4X4). Luma uses the
        // block's intra mode; chroma is DC_PRED, plus the CfL high-frequency term when selected.
        // Luma (plane 0) is reconstructed first, so the chroma CfL reads finalized luma recon.
        for plane in 0..3 {
            let pred = Pred {
                mode: if plane == 0 { y_mode } else { DC_PRED },
                filter_intra: if plane == 0 { filter_intra } else { None },
                cfl_alpha: match (plane, cfl) {
                    (1, Some((au, _))) => Some(au),
                    (2, Some((_, av))) => Some(av),
                    _ => None,
                },
            };
            for ty in 0..n4 {
                for tx in 0..n4 {
                    let sx = c * 4 + tx * 4;
                    let sy = r * 4 + ty * 4;
                    if sx >= self.coded_w || sy >= self.coded_h {
                        continue; // transform block entirely outside the frame
                    }
                    self.transform_block(plane, sx, sy, bw, pred);
                }
            }
        }

        // Mark this block's 4×4 cells decoded (`BlockDecoded`, §5.11.34) so later neighbours see the
        // correct above-right / below-left availability for directional prediction.
        for ty in 0..n4 {
            for tx in 0..n4 {
                let by = (r + ty) as isize - self.sb_r as isize;
                let bx = (c + tx) as isize - self.sb_c as isize;
                if (0..16).contains(&by) && (0..16).contains(&bx) {
                    self.block_decoded[(by + 1) as usize * BD_STRIDE + (bx + 1) as usize] = 1;
                }
            }
        }
    }

    fn transform_block(&mut self, plane: usize, sx: usize, sy: usize, block_w: usize, desc: Pred) {
        let pred = match desc.filter_intra {
            Some(fi) if plane == 0 => self.predict_filter_intra_4x4(plane, sx, sy, fi),
            _ => {
                // Chroma DC base prediction, then (for CfL) add the alpha-scaled luma high-frequency
                // from the *reconstructed* luma of this block (§7.11.5).
                let mut p = self.predict_4x4(plane, sx, sy, desc.mode);
                if let Some(alpha) = desc.cfl_alpha {
                    self.apply_cfl(&mut p, sx, sy, alpha);
                }
                p
            }
        };
        let mut res = [0i32; 16];
        for i in 0..4 {
            for j in 0..4 {
                res[i * 4 + j] = self.sample(plane, sx + j, sy + i) - pred[i * 4 + j];
            }
        }
        if self.qindex == 0 {
            let quant = gamut_dsp::fwht4x4(&res);
            self.code_coeffs(plane, sx >> 2, sy >> 2, block_w, &quant, 1);
            return;
        }

        // Lossy: pick the transform type (luma only — chroma is forced DCT_DCT, which is not
        // signaled), forward-transform + quantize, code, then reconstruct exactly as the decoder
        // will and store into the reconstruction buffer for later prediction. Because the recon is
        // `pred + inverse(dequant(levels))` and the decoder runs the same inverse on the same
        // levels, the result is bit-exact for whichever transform type is signaled.
        let (tx, tx_sym, levels) = if plane == 0 {
            self.select_tx_type(&res)
        } else {
            (TxType::DctDct, 1, self.quantize_tx(&res, TxType::DctDct))
        };
        self.code_coeffs(plane, sx >> 2, sy >> 2, block_w, &levels, tx_sym);

        let mut dq = [0i32; 16];
        for (i, &lvl) in levels.iter().enumerate() {
            let q = if i == 0 { self.dc_quant } else { self.ac_quant };
            dq[i] = dequant(lvl, q, 1, 8);
        }
        let resid = inverse_transform_2d(&dq, TxSize::Tx4x4, tx, 8);
        for i in 0..4 {
            for j in 0..4 {
                let v = (pred[i * 4 + j] + resid[i * 4 + j]).clamp(0, 255) as u8;
                self.recon[plane][(sy + i) * self.coded_w + (sx + j)] = v;
            }
        }
    }

    /// Forward-transforms and quantizes a 4×4 residual under transform type `tx`, returning the
    /// 16 quantized coefficient levels (DC uses `dc_quant`, AC uses `ac_quant`).
    fn quantize_tx(&self, res: &[i32; 16], tx: TxType) -> [i32; 16] {
        let coeff = forward_transform_2d(res, TxSize::Tx4x4, tx);
        let mut levels = [0i32; 16];
        for (i, &c) in coeff.iter().enumerate() {
            let q = if i == 0 { self.dc_quant } else { self.ac_quant };
            levels[i] = quantize(c, q);
        }
        levels
    }

    /// Selects a luma 4×4 transform type from `TX_SET_INTRA_2` by a simple coded-cost proxy (sum of
    /// `1 + |level|` over non-zero coefficients), preferring `DCT_DCT` on ties. Returns the type,
    /// its `Tx_Type_Intra_Inv_Set2` symbol, and the quantized levels. The choice is a quality
    /// decision (any valid type round-trips bit-exactly); only the *signaling* must be correct.
    fn select_tx_type(&self, res: &[i32; 16]) -> (TxType, usize, [i32; 16]) {
        let cost = |levels: &[i32; 16]| -> i32 {
            levels
                .iter()
                .map(|&l| if l == 0 { 0 } else { 1 + l.abs() })
                .sum()
        };
        // Start from DCT_DCT (symbol 1) so it wins ties; the rest of TX_SET_INTRA_2 follows.
        let mut best = (
            TxType::DctDct,
            1usize,
            self.quantize_tx(res, TxType::DctDct),
        );
        let mut best_cost = cost(&best.2);
        for (tx, sym) in [
            (TxType::AdstAdst, 2usize),
            (TxType::AdstDct, 3),
            (TxType::DctAdst, 4),
            (TxType::Idtx, 0),
        ] {
            let levels = self.quantize_tx(res, tx);
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
        let nb = |x: usize, y: usize| -> i32 {
            if self.qindex > 0 {
                i32::from(self.recon[plane][y * self.coded_w + x])
            } else {
                self.sample(plane, x, y)
            }
        };
        let have_above = sy > 0;
        let have_left = sx > 0;
        match (have_above, have_left) {
            (true, true) => {
                let mut s = 0;
                for k in 0..4 {
                    s += nb(sx + k, sy - 1);
                    s += nb(sx - 1, sy + k);
                }
                (s + 4) >> 3
            }
            (false, true) => {
                let mut s = 0;
                for k in 0..4 {
                    s += nb(sx - 1, sy + k);
                }
                (s + 2) >> 2
            }
            (true, false) => {
                let mut s = 0;
                for k in 0..4 {
                    s += nb(sx + k, sy - 1);
                }
                (s + 2) >> 2
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
        let have_left = sx > 0;

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

    /// Builds the 4×4 recursive filter-intra prediction (§7.11.2.3) for luma at coded `(sx, sy)`
    /// under `filter_intra_mode`. With `w == h == 4` the spec's `w4 = 1`, `h2 = 2`: two stacked 4×2
    /// sub-blocks, the lower filtering the predicted samples of the upper. Each output 4×2 weights a
    /// 7-sample neighbourhood `p` by [`cdf::INTRA_FILTER_TAPS`] and scales by `INTRA_FILTER_SCALE_BITS
    /// = 4`. Only `AboveRow[-1..3]` / `LeftCol[-1..3]` are read, so no above-right / below-left
    /// extension is needed. Used only on the lossy path, where `enable_filter_intra = 1`.
    fn predict_filter_intra_4x4(
        &self,
        plane: usize,
        sx: usize,
        sy: usize,
        fi_mode: u8,
    ) -> [i32; 16] {
        let (above, left, top_left) = self.reference_4x4(plane, sx, sy);
        let above_row = |k: i32| -> i32 { if k < 0 { top_left } else { above[k as usize] } };
        let left_col = |k: i32| -> i32 { if k < 0 { top_left } else { left[k as usize] } };
        let taps = &cdf::INTRA_FILTER_TAPS[fi_mode as usize];
        let mut pred = [0i32; 16];
        // For each 4×2 sub-block (i2 = 0..1; j4 is always 0 since w4 = 1).
        for i2 in 0..2i32 {
            // p[0..6]: the 7 neighbouring samples (§7.11.2.3). The top sub-block reads AboveRow; the
            // bottom one reads the top sub-block's predicted row 1 plus LeftCol.
            let mut p = [0i32; 7];
            for (i, pi) in p.iter_mut().enumerate() {
                let i = i as i32;
                *pi = if i < 5 {
                    if i2 == 0 {
                        above_row(i - 1)
                    } else if i == 0 {
                        left_col((i2 << 1) - 1)
                    } else {
                        pred[(((i2 << 1) - 1) * 4 + (i - 1)) as usize]
                    }
                } else {
                    left_col((i2 << 1) + i - 5)
                };
            }
            for i1 in 0..2i32 {
                for j1 in 0..4i32 {
                    let row = ((i1 << 2) + j1) as usize;
                    let mut pr = 0i32;
                    for (k, &tap) in taps[row].iter().enumerate() {
                        pr += i32::from(tap) * p[k];
                    }
                    let idx = (((i2 << 1) + i1) * 4 + j1) as usize;
                    pred[idx] = round2_signed(pr, 4).clamp(0, 255);
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

    /// Picks the lossy luma prediction minimizing SAD against the source 4×4 at `(sx, sy)`. Returns
    /// the `YMode` to signal and, when recursive filter-intra (§7.11.2.3) beats every regular mode,
    /// the chosen `filter_intra_mode` (signaled as `YMode = DC_PRED` + `use_filter_intra = 1`).
    /// Regular modes win ties (and `DC_PRED` wins among them), so the cheaper signaling is preferred.
    /// The choice is a quality decision; every option reconstructs bit-exactly, so only the
    /// signaling must be correct.
    fn select_luma_mode(&self, sx: usize, sy: usize) -> (u8, Option<u8>) {
        let sad = |pred: &[i32; 16]| -> i32 {
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
            let s = sad(&self.predict_filter_intra_4x4(0, sx, sy, fi));
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

    /// Adds the chroma-from-luma high-frequency term to a 4×4 chroma DC prediction in place
    /// (§7.11.5). For 4:4:4 the subsampled luma `L[i][j]` is just the reconstructed luma sample
    /// (`× 8`, i.e. 3 fractional bits); `lumaAvg = Round2(ΣL, 4)`. Each chroma sample becomes
    /// `Clip1(dc + Round2Signed(alpha * (L - lumaAvg), 6))`. `alpha == 0` is a no-op (plain DC).
    fn apply_cfl(&self, pred: &mut [i32; 16], sx: usize, sy: usize, alpha: i32) {
        let mut l = [0i32; 16];
        let mut sum = 0i32;
        for i in 0..4 {
            for j in 0..4 {
                let v = i32::from(self.recon[0][(sy + i) * self.coded_w + (sx + j)]) << 3;
                l[i * 4 + j] = v;
                sum += v;
            }
        }
        let luma_avg = (sum + 8) >> 4; // Round2(sum, Tx_Width_Log2 + Tx_Height_Log2) = Round2(_, 4)
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
    fn select_cfl(&self, sx: usize, sy: usize) -> Option<(i32, i32)> {
        // Source luma high-frequency (matching apply_cfl's reconstructed-luma formula).
        let mut l = [0i32; 16];
        let mut sum = 0i32;
        for i in 0..4 {
            for j in 0..4 {
                let v = self.sample(0, sx + j, sy + i) << 3;
                l[i * 4 + j] = v;
                sum += v;
            }
        }
        let luma_avg = (sum + 8) >> 4;

        let best_alpha = |plane: usize| -> i32 {
            let dc = self.dc_avg(plane, sx, sy);
            let sad = |alpha: i32| -> i32 {
                let mut s = 0;
                for i in 0..4 {
                    for j in 0..4 {
                        let pred = (dc + round2_signed(alpha * (l[i * 4 + j] - luma_avg), 6))
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
    fn code_coeffs(
        &mut self,
        plane: usize,
        x4: usize,
        y4: usize,
        block_w: usize,
        quant: &[i32; 16],
        tx_sym: usize,
    ) {
        let ptype = usize::from(plane > 0);
        let qctx = self.qctx;
        let scan = &cdf::DEFAULT_SCAN_4X4;

        let mut eob = 0usize;
        for c in 0..16 {
            if quant[scan[c]] != 0 {
                eob = c + 1;
            }
        }

        let txb_ctx = self.txb_skip_ctx(plane, x4, y4, block_w);
        self.sym
            .encode_symbol(usize::from(eob == 0), &cdf::TXB_SKIP[qctx][txb_ctx]);
        if eob == 0 {
            self.set_ctx(plane, x4, y4, 0, 0);
            return;
        }

        // transform_type (§5.11.39): signaled only for luma when not all-zero and base_q_idx > 0.
        // Reduced tx set + 4×4 intra ⇒ TX_SET_INTRA_2; `tx_sym` indexes
        // Tx_Type_Intra_Inv_Set2 = {IDTX, DCT_DCT, ADST_ADST, ADST_DCT, DCT_ADST}.
        if self.qindex > 0 && plane == 0 {
            self.sym.encode_symbol(tx_sym, &cdf::INTRA_TX_TYPE_SET2_4X4);
        }

        // eob position (TX_CLASS_2D ⇒ eob_pt context 0).
        let eobpt = eobpt_from_eob(eob);
        self.sym
            .encode_symbol(eobpt - 1, &cdf::EOB_PT_16[qctx][ptype][0]);
        if eobpt >= 3 {
            let nbits = eobpt - 2;
            let base = (1usize << (eobpt - 2)) + 1;
            let extra = eob - base;
            self.sym.encode_symbol(
                (extra >> (nbits - 1)) & 1,
                &cdf::EOB_EXTRA[qctx][ptype][eobpt - 3],
            );
            let mut i = nbits as isize - 2;
            while i >= 0 {
                self.sym.encode_literal(((extra >> i) & 1) as u32, 1);
                i -= 1;
            }
        }

        // Base levels + base range, scanned from the last coefficient back to DC.
        let mut levels = [0i32; 16];
        for c in (0..eob).rev() {
            let pos = scan[c];
            let level = quant[pos].abs();
            if c == eob - 1 {
                let ctx = coeff_base_eob_ctx(c);
                self.sym.encode_symbol(
                    (level.min(3) - 1) as usize,
                    &cdf::COEFF_BASE_EOB[qctx][ptype][ctx],
                );
            } else {
                let ctx = coeff_base_ctx(pos, &levels);
                self.sym
                    .encode_symbol(level.min(3) as usize, &cdf::COEFF_BASE[qctx][ptype][ctx]);
            }
            if level > NUM_BASE_LEVELS {
                let br_ctx = coeff_br_ctx(pos, &levels);
                let mut rem = level - 3;
                for _ in 0..4 {
                    let brv = rem.min(3);
                    self.sym
                        .encode_symbol(brv as usize, &cdf::COEFF_BR[qctx][ptype][br_ctx]);
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
                    let ctx = self.dc_sign_ctx(plane, x4, y4);
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
        self.set_ctx(plane, x4, y4, cul, dc_cat);
    }

    fn set_ctx(&mut self, plane: usize, x4: usize, y4: usize, cul: u8, dc: u8) {
        self.above_level[plane][x4] = cul;
        self.above_dc[plane][x4] = dc;
        self.left_level[plane][y4] = cul;
        self.left_dc[plane][y4] = dc;
    }

    fn txb_skip_ctx(&self, plane: usize, x4: usize, y4: usize, block_w: usize) -> usize {
        if plane == 0 {
            if block_w == 4 {
                return 0;
            }
            let top = i32::from(self.above_level[0][x4]);
            let left = i32::from(self.left_level[0][y4]);
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
            let above = self.above_level[plane][x4] | self.above_dc[plane][x4];
            let left = self.left_level[plane][y4] | self.left_dc[plane][y4];
            let mut ctx = usize::from(above != 0) + usize::from(left != 0) + 7;
            if block_w * block_w > 16 {
                ctx += 3;
            }
            ctx
        }
    }

    fn dc_sign_ctx(&self, plane: usize, x4: usize, y4: usize) -> usize {
        let mut s = 0i32;
        for &cat in &[self.above_dc[plane][x4], self.left_dc[plane][y4]] {
            if cat == 1 {
                s -= 1;
            } else if cat == 2 {
                s += 1;
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

fn coeff_base_eob_ctx(c: usize) -> usize {
    if c == 0 {
        0
    } else if c <= 2 {
        1
    } else if c <= 4 {
        2
    } else {
        3
    }
}

fn coeff_base_ctx(pos: usize, levels: &[i32; 16]) -> usize {
    let (row, col) = (pos >> 2, pos & 3);
    let mut mag = 0i32;
    for &(dr, dc) in &cdf::SIG_REF_DIFF_OFFSET_2D {
        let (rr, cc) = (row + dr, col + dc);
        if rr < 4 && cc < 4 {
            mag += levels[(rr << 2) + cc].abs().min(3);
        }
    }
    let ctx = (((mag + 1) >> 1).min(4)) as usize;
    if row == 0 && col == 0 {
        return 0;
    }
    ctx + usize::from(cdf::COEFF_BASE_CTX_OFFSET_4X4[row.min(4)][col.min(4)])
}

fn coeff_br_ctx(pos: usize, levels: &[i32; 16]) -> usize {
    let (row, col) = (pos >> 2, pos & 3);
    let mut mag = 0i32;
    for &(dr, dc) in &cdf::MAG_REF_OFFSET_2D {
        let (rr, cc) = (row + dr, col + dc);
        if rr < 4 && cc < 4 {
            mag += levels[(rr << 2) + cc].abs().min(15);
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

/// `Round2Signed(x, n)` (§4.7): symmetric rounding right shift (rounds the magnitude, keeps sign).
fn round2_signed(x: i32, n: u32) -> i32 {
    if x >= 0 {
        (x + (1 << (n - 1))) >> n
    } else {
        -((-x + (1 << (n - 1))) >> n)
    }
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
    fn tx_type_selection_matches_set2_mapping() {
        let p = grey4x4();
        let e = FrameEncoder::new(&p, 48);

        // A flat residual concentrates into the DCT DC bin (one coefficient), so DCT_DCT wins and
        // is signaled as Tx_Type_Intra_Inv_Set2 symbol 1.
        let (tx, sym, _) = e.select_tx_type(&[40i32; 16]);
        assert!(matches!(tx, TxType::DctDct));
        assert_eq!(sym, 1);

        // A single non-DC impulse stays a single coefficient under the identity transform but
        // spreads across many DCT bins, so IDTX wins and is signaled as symbol 0.
        let mut impulse = [0i32; 16];
        impulse[5] = 220;
        let (tx, sym, levels) = e.select_tx_type(&impulse);
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
        let a = e.select_tx_type(&res);
        let b = e.select_tx_type(&res);
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
        for (j, &v) in [60u8, 90, 120, 150].iter().enumerate() {
            e.recon[0][3 * cw + (4 + j)] = v;
        }
        for (i, &v) in [40u8, 80, 120, 160].iter().enumerate() {
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
        let pred = e.predict_filter_intra_4x4(0, 4, 4, 0);
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
                e.predict_filter_intra_4x4(0, 4, 4, fi),
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
                e.recon[0][(4 + i) * cw + (4 + j)] = (80 + 8 * j) as u8;
            }
        }
        let mut pred = [128i32; 16];
        e.apply_cfl(&mut pred, 4, 4, 2);
        assert_eq!(
            pred,
            [
                125, 127, 129, 131, 125, 127, 129, 131, 125, 127, 129, 131, 125, 127, 129, 131
            ]
        );
        // alpha = 0 is a no-op (plain DC).
        let mut flat = [100i32; 16];
        e.apply_cfl(&mut flat, 4, 4, 0);
        assert_eq!(flat, [100; 16]);
    }

    #[test]
    fn d45_reads_above_right_when_available() {
        // Block at (4,4) ⇒ superblock-relative (by, bx) = (1, 1). Mark the above-right 4×4 decoded
        // so D45 (pAngle 45) reads the real extended above-row samples 4..8.
        let p = Planar8::from_rgb8_identity(&[128u8; 12 * 12 * 3], 12, 12).unwrap();
        let mut e = FrameEncoder::new(&p, 32);
        let cw = e.coded_w;
        for (k, &v) in [10u8, 20, 30, 40, 50, 60, 70, 80].iter().enumerate() {
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
