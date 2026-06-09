//! The single-tile, all-intra encoder: superblock/partition iteration (§5.11.2/.4), DC intra
//! prediction (§7.11.2.5), the forward transform (via `gamut-dsp`), and coefficient coding with
//! full context derivation (§5.11.39, §8.3.2).
//!
//! Two paths share this code, selected by `qindex` (`base_q_idx`):
//! - **Lossless** (`qindex == 0`): forced `TX_4X4` Walsh–Hadamard; prediction neighbours are the
//!   source samples (which equal the reconstruction under lossless); partitions are
//!   `PARTITION_NONE` except at the right/bottom frame edges.
//! - **Lossy** (`qindex > 0`): `TX_4X4` with quantization. Blocks are forced all-4×4
//!   (`PARTITION_SPLIT` everywhere) so each uses a 4×4 transform under `TX_MODE_LARGEST` with no
//!   tx-depth signaling. The luma transform type is chosen per block from `TX_SET_INTRA_2`
//!   (`{IDTX, DCT_DCT, ADST_ADST, ADST_DCT, DCT_ADST}`) and signaled; chroma is `DCT_DCT`.
//!   Prediction reads a
//!   **reconstruction buffer** that the encoder maintains exactly as the decoder would (predict →
//!   transform → quantize → dequantize → inverse-transform → add → store), so the encoder's
//!   reconstruction is bit-exact with a conformant decoder's output.
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
        }
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
                self.encode_partition(r, c, 64);
                c += SB4;
            }
            r += SB4;
        }
        let recon = Reconstruction {
            planes: self.recon,
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

        // intra_frame_mode_info: skip=0 (ctx 0), y_mode=DC_PRED (ctx [0][0]), uv_mode=DC_PRED.
        self.sym.encode_symbol(0, &cdf::SKIP[0]);
        self.sym.encode_symbol(0, &cdf::INTRA_FRAME_Y_MODE_DC_DC);
        let uv: &[u16] = if bw == 4 {
            &cdf::UV_MODE_CFL_ALLOWED_DC
        } else {
            &cdf::UV_MODE_CFL_NOT_ALLOWED_DC
        };
        self.sym.encode_symbol(0, uv);

        for y in 0..n4 {
            for x in 0..n4 {
                let (rr, cc) = (r + y, c + x);
                if rr < self.mi_rows && cc < self.mi_cols {
                    self.mi_bsl[rr * self.mi_cols + cc] = bsl;
                }
            }
        }

        // residual(): per plane, raster of 4×4 transform blocks (Lossless ⇒ TX_4X4).
        for plane in 0..3 {
            for ty in 0..n4 {
                for tx in 0..n4 {
                    let sx = c * 4 + tx * 4;
                    let sy = r * 4 + ty * 4;
                    if sx >= self.coded_w || sy >= self.coded_h {
                        continue; // transform block entirely outside the frame
                    }
                    self.transform_block(plane, sx, sy, bw);
                }
            }
        }
    }

    fn transform_block(&mut self, plane: usize, sx: usize, sy: usize, block_w: usize) {
        let avg = self.dc_avg(plane, sx, sy);
        let mut res = [0i32; 16];
        for i in 0..4 {
            for j in 0..4 {
                res[i * 4 + j] = self.sample(plane, sx + j, sy + i) - avg;
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
                let v = (avg + resid[i * 4 + j]).clamp(0, 255) as u8;
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
            self.sym
                .encode_symbol(tx_sym, &cdf::INTRA_TX_TYPE_SET2_4X4_DC);
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
}
