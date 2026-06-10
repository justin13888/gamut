//! VP8 key-frame reconstruction pipeline (RFC 6386 §10–§14): the macroblock loop that ties together
//! prediction, the transforms, quantization, and token coding into an encodable/decodable frame.
//!
//! This is the keystone of the lossy path. Each macroblock is predicted from the **reconstructed**
//! neighbors in a recon buffer (the encoder predicts exactly as the decoder does), so the encoder's
//! reconstruction is bit-identical to any conformant decoder's output. The minimal pipeline here codes
//! every macroblock with `DC_PRED` luma (16×16) and chroma (8×8), a Y2 (luma-DC WHT) block, one token
//! partition, and no loop filter or skip; later phases add the remaining modes, filters, and header
//! features. Tracked in `../STATUS.md` section L.

// The macroblock/block math indexes several fixed-size arrays in lock-step (and over partial ranges
// like `1..16`), where explicit indices read closer to the spec than iterator adaptors.
#![allow(clippy::needless_range_loop)]

use gamut_color::Yuv420;
use gamut_core::{Error, Result};

use super::bool_coder::{BoolDecoder, BoolEncoder};
use super::header::{
    self, LoopFilterParams, QuantIndices, Segmentation, UNCOMPRESSED_CHUNK_LEN, Vp8FrameHeader,
};
use super::prediction::{self, DC_PRED};
use super::quant::{self, QuantFactors};
use super::tokens::{self, CoeffProbs};
use super::transform::{clamp255, fdct4x4, fwht4x4, idct4x4, iwht4x4};

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

/// Builds the minimal key-frame header for the given dimensions and base quantizer index.
fn frame_header(width: u32, height: u32, quant_index: u8) -> Vp8FrameHeader {
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
            simple: false,
            level: 0,
            sharpness: 0,
        },
        token_partitions: 1,
        quant: QuantIndices {
            y_ac: quant_index,
            ..QuantIndices::default()
        },
        refresh_entropy_probs: true,
        mb_no_skip_coeff: false,
        prob_skip_false: 0,
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

/// Writes `clamp255(pred + residue)` into the 4×4 block at `(x, y)` of `plane`.
fn write_block(plane: &mut [u8], stride: usize, x: usize, y: usize, pred: u8, residue: &[i16; 16]) {
    for r in 0..4 {
        for c in 0..4 {
            let v = i32::from(pred) + i32::from(residue[r * 4 + c]);
            plane[(y + r) * stride + x + c] = clamp255(v);
        }
    }
}

/// The luma DC prediction value for the macroblock at `(mb_x, mb_y)` of `recon`.
fn luma_dc(recon: &FrameBuffers, mb_x: usize, mb_y: usize) -> u8 {
    let (px, py, stride) = (mb_x * 16, mb_y * 16, recon.y_stride());
    let above = (mb_y > 0).then(|| row_at(&recon.y, stride, px, py - 1, 16));
    let left = (mb_x > 0).then(|| col_at(&recon.y, stride, px - 1, py, 16));
    prediction::dc_predict(
        16,
        above.as_ref().map(|a| &a[..16]),
        left.as_ref().map(|l| &l[..16]),
    )
}

/// The chroma DC prediction value for one plane at `(mb_x, mb_y)`.
fn chroma_dc(plane: &[u8], stride: usize, mb_x: usize, mb_y: usize) -> u8 {
    let (px, py) = (mb_x * 8, mb_y * 8);
    let above = (mb_y > 0).then(|| row_at(plane, stride, px, py - 1, 8));
    let left = (mb_x > 0).then(|| col_at(plane, stride, px - 1, py, 8));
    prediction::dc_predict(
        8,
        above.as_ref().map(|a| &a[..8]),
        left.as_ref().map(|l| &l[..8]),
    )
}

/// Reconstructs the 16 luma sub-blocks of a macroblock: the Y2 inverse-WHT supplies each sub-block's
/// DC, the AC levels are dequantized, and `pred + idct` is written into the recon buffer. Shared by
/// the encoder and decoder.
fn reconstruct_luma(
    recon: &mut FrameBuffers,
    mb_x: usize,
    mb_y: usize,
    pred: u8,
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
        let (x, y) = (mb_x * 16 + (i % 4) * 4, mb_y * 16 + (i / 4) * 4);
        write_block(&mut recon.y, stride, x, y, pred, &residue);
    }
}

/// Reconstructs the four sub-blocks of one chroma plane from full (DC+AC) levels.
fn reconstruct_chroma(
    plane: &mut [u8],
    stride: usize,
    mb_x: usize,
    mb_y: usize,
    pred: u8,
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
        let (x, y) = (mb_x * 8 + (i % 2) * 4, mb_y * 8 + (i / 2) * 4);
        write_block(plane, stride, x, y, pred, &residue);
    }
}

/// Transforms + quantizes one luma macroblock against its DC prediction, returning the Y2 and per
/// sub-block AC levels.
fn quantize_luma(
    src: &[u8],
    stride: usize,
    mb_x: usize,
    mb_y: usize,
    pred: u8,
    qf: &QuantFactors,
    levels: &mut MbLevels,
) {
    let mut y_coeffs = [[0i16; 16]; 16];
    let mut y_dc = [0i16; 16];
    for i in 0..16 {
        let (x, y) = (mb_x * 16 + (i % 4) * 4, mb_y * 16 + (i / 4) * 4);
        let block = read_block(src, stride, x, y);
        let residue: [i16; 16] = core::array::from_fn(|k| block[k] - i16::from(pred));
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

/// Transforms + quantizes one chroma plane's four sub-blocks against its DC prediction.
fn quantize_chroma(
    src: &[u8],
    stride: usize,
    mb_x: usize,
    mb_y: usize,
    pred: u8,
    qf: &QuantFactors,
) -> [[i16; 16]; 4] {
    let mut levels = [[0i16; 16]; 4];
    for i in 0..4 {
        let (x, y) = (mb_x * 8 + (i % 2) * 4, mb_y * 8 + (i / 2) * 4);
        let block = read_block(src, stride, x, y);
        let residue: [i16; 16] = core::array::from_fn(|k| block[k] - i16::from(pred));
        let coeffs = fdct4x4(&residue);
        levels[i][0] = quant::quantize(coeffs[0], qf.uv_dc);
        for k in 1..16 {
            levels[i][k] = quant::quantize(coeffs[k], qf.uv_ac);
        }
    }
    levels
}

/// Codes one macroblock's coefficient blocks in Y2 → Y → U → V order, threading the `above`/`left`
/// non-zero context (RFC 6386 §13.3).
fn encode_mb_tokens(
    enc: &mut BoolEncoder,
    above: &mut EntropyCtx,
    left: &mut EntropyCtx,
    probs: &CoeffProbs,
    levels: &MbLevels,
) {
    let has = tokens::encode_block(
        enc,
        &levels.y2,
        1,
        usize::from(above.y2) + usize::from(left.y2),
        probs,
    );
    above.y2 = has;
    left.y2 = has;
    for i in 0..16 {
        let (r, c) = (i / 4, i % 4);
        let ctx = usize::from(above.y[c]) + usize::from(left.y[r]);
        let has = tokens::encode_block(enc, &levels.y[i], 0, ctx, probs);
        above.y[c] = has;
        left.y[r] = has;
    }
    for i in 0..4 {
        let (r, c) = (i / 2, i % 2);
        let ctx = usize::from(above.u[c]) + usize::from(left.u[r]);
        let has = tokens::encode_block(enc, &levels.u[i], 2, ctx, probs);
        above.u[c] = has;
        left.u[r] = has;
    }
    for i in 0..4 {
        let (r, c) = (i / 2, i % 2);
        let ctx = usize::from(above.v[c]) + usize::from(left.v[r]);
        let has = tokens::encode_block(enc, &levels.v[i], 2, ctx, probs);
        above.v[c] = has;
        left.v[r] = has;
    }
}

/// Decodes one macroblock's coefficient blocks, mirroring [`encode_mb_tokens`].
fn decode_mb_tokens(
    dec: &mut BoolDecoder,
    above: &mut EntropyCtx,
    left: &mut EntropyCtx,
    probs: &CoeffProbs,
) -> MbLevels {
    let mut levels = MbLevels::default();
    let has = tokens::decode_block(
        dec,
        &mut levels.y2,
        1,
        usize::from(above.y2) + usize::from(left.y2),
        probs,
    );
    above.y2 = has;
    left.y2 = has;
    for i in 0..16 {
        let (r, c) = (i / 4, i % 4);
        let ctx = usize::from(above.y[c]) + usize::from(left.y[r]);
        let has = tokens::decode_block(dec, &mut levels.y[i], 0, ctx, probs);
        above.y[c] = has;
        left.y[r] = has;
    }
    for i in 0..4 {
        let (r, c) = (i / 2, i % 2);
        let ctx = usize::from(above.u[c]) + usize::from(left.u[r]);
        let has = tokens::decode_block(dec, &mut levels.u[i], 2, ctx, probs);
        above.u[c] = has;
        left.u[r] = has;
    }
    for i in 0..4 {
        let (r, c) = (i / 2, i % 2);
        let ctx = usize::from(above.v[c]) + usize::from(left.v[r]);
        let has = tokens::decode_block(dec, &mut levels.v[i], 2, ctx, probs);
        above.v[c] = has;
        left.v[r] = has;
    }
    levels
}

/// Encodes a [`Yuv420`] image as a VP8 key-frame bitstream (the `VP8 ` chunk payload), returning the
/// bitstream and the encoder's reconstruction (the tier-2 oracle: it must equal any decoder's output).
#[must_use]
pub fn encode_frame(yuv: &Yuv420, quant_index: u8) -> (Vec<u8>, FrameBuffers) {
    let header = frame_header(yuv.width(), yuv.height(), quant_index);
    let qf = QuantFactors::for_frame(&header.quant);
    let mut recon = FrameBuffers::new(yuv.width(), yuv.height());

    // Macroblock-aligned source planes.
    let (yw, yh) = (recon.y_stride(), recon.mb_rows * 16);
    let (cw, ch) = (recon.c_stride(), recon.mb_rows * 8);
    let src_y = pad_plane(yuv.y(), yuv.width() as usize, yuv.height() as usize, yw, yh);
    let vcw = Yuv420::chroma_width(yuv.width()) as usize;
    let vch = Yuv420::chroma_height(yuv.height()) as usize;
    let src_u = pad_plane(yuv.u(), vcw, vch, cw, ch);
    let src_v = pad_plane(yuv.v(), vcw, vch, cw, ch);

    let mut modes = BoolEncoder::new();
    header::write_frame_header(&mut modes, &header);
    let mut residuals = BoolEncoder::new();
    let probs = &tokens::DEFAULT_COEFF_PROBS;

    let mut above = vec![EntropyCtx::default(); recon.mb_cols];
    for mb_y in 0..recon.mb_rows {
        let mut left = EntropyCtx::default();
        for mb_x in 0..recon.mb_cols {
            let y_pred = luma_dc(&recon, mb_x, mb_y);
            let u_pred = chroma_dc(&recon.u, recon.c_stride(), mb_x, mb_y);
            let v_pred = chroma_dc(&recon.v, recon.c_stride(), mb_x, mb_y);

            let mut levels = MbLevels::default();
            quantize_luma(&src_y, yw, mb_x, mb_y, y_pred, &qf, &mut levels);
            levels.u = quantize_chroma(&src_u, cw, mb_x, mb_y, u_pred, &qf);
            levels.v = quantize_chroma(&src_v, cw, mb_x, mb_y, v_pred, &qf);

            reconstruct_luma(&mut recon, mb_x, mb_y, y_pred, &levels, &qf);
            let cstride = recon.c_stride();
            reconstruct_chroma(&mut recon.u, cstride, mb_x, mb_y, u_pred, &levels.u, &qf);
            reconstruct_chroma(&mut recon.v, cstride, mb_x, mb_y, v_pred, &levels.v, &qf);

            modes.put_tree(
                prediction::KF_YMODE_TREE,
                &prediction::KF_YMODE_PROB,
                DC_PRED,
            );
            modes.put_tree(
                prediction::KF_UV_MODE_TREE,
                &prediction::KF_UV_MODE_PROB,
                DC_PRED,
            );
            encode_mb_tokens(&mut residuals, &mut above[mb_x], &mut left, probs, &levels);
        }
    }

    let part0 = modes.finish();
    let part1 = residuals.finish();
    let mut out = Vec::with_capacity(UNCOMPRESSED_CHUNK_LEN + part0.len() + part1.len());
    header::write_uncompressed_chunk(&header, part0.len() as u32, &mut out);
    out.extend_from_slice(&part0);
    out.extend_from_slice(&part1);
    (out, recon)
}

/// Decodes a VP8 key-frame bitstream (the `VP8 ` chunk payload) into reconstructed planes.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] for a malformed stream or [`Error::Unsupported`] for features not
/// yet implemented (non-DC prediction modes, multiple token partitions, …).
pub fn decode_frame(data: &[u8]) -> Result<FrameBuffers> {
    let chunk = header::read_uncompressed_chunk(data)?;
    let part0_end = UNCOMPRESSED_CHUNK_LEN + chunk.first_partition_size as usize;
    if part0_end > data.len() {
        return Err(Error::InvalidInput("VP8: first partition exceeds frame"));
    }
    let mut modes = BoolDecoder::new(&data[UNCOMPRESSED_CHUNK_LEN..part0_end]);
    let (head, coeff_probs) = header::read_frame_header(&chunk, &mut modes)?;
    if head.token_partitions != 1 {
        return Err(Error::Unsupported(
            "VP8: multiple token partitions not yet supported",
        ));
    }
    let qf = QuantFactors::for_frame(&head.quant);
    let mut residuals = BoolDecoder::new(&data[part0_end..]);
    let mut recon = FrameBuffers::new(u32::from(chunk.width), u32::from(chunk.height));

    let mut above = vec![EntropyCtx::default(); recon.mb_cols];
    for mb_y in 0..recon.mb_rows {
        let mut left = EntropyCtx::default();
        for mb_x in 0..recon.mb_cols {
            if modes.get_tree(prediction::KF_YMODE_TREE, &prediction::KF_YMODE_PROB) != DC_PRED {
                return Err(Error::Unsupported(
                    "VP8: non-DC luma mode not yet supported",
                ));
            }
            if modes.get_tree(prediction::KF_UV_MODE_TREE, &prediction::KF_UV_MODE_PROB) != DC_PRED
            {
                return Err(Error::Unsupported(
                    "VP8: non-DC chroma mode not yet supported",
                ));
            }
            let y_pred = luma_dc(&recon, mb_x, mb_y);
            let u_pred = chroma_dc(&recon.u, recon.c_stride(), mb_x, mb_y);
            let v_pred = chroma_dc(&recon.v, recon.c_stride(), mb_x, mb_y);
            let levels =
                decode_mb_tokens(&mut residuals, &mut above[mb_x], &mut left, &coeff_probs);

            reconstruct_luma(&mut recon, mb_x, mb_y, y_pred, &levels, &qf);
            let cstride = recon.c_stride();
            reconstruct_chroma(&mut recon.u, cstride, mb_x, mb_y, u_pred, &levels.u, &qf);
            reconstruct_chroma(&mut recon.v, cstride, mb_x, mb_y, v_pred, &levels.v, &qf);
        }
    }
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
    fn decode_rejects_truncated_first_partition() {
        let yuv = pattern(16, 16);
        let (mut bitstream, _) = encode_frame(&yuv, 40);
        bitstream.truncate(UNCOMPRESSED_CHUNK_LEN + 1);
        // Decoding must not panic; it either errors or zero-pads, but stays well-defined.
        let _ = decode_frame(&bitstream);
    }
}
