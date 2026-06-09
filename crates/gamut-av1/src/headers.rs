//! AV1 OBU framing (§5.3, Annex B), the reduced-still-picture sequence header (§5.5), and the
//! lossless intra-keyframe uncompressed frame header (§5.9.2), specialised to the M0 config.

use gamut_bitstream::{BitWriter, write_leb128};
use gamut_core::{Error, Result};

/// `OBU_SEQUENCE_HEADER`.
const OBU_SEQUENCE_HEADER: u8 = 1;
/// `OBU_FRAME` (frame header + tile group in one OBU).
pub(crate) const OBU_FRAME: u8 = 6;

/// The sequence-header field values that `gamut-avif` must mirror into `av1C` and `colr`
/// (AV1-ISOBMFF v1.3.0 §2.3.4). Fixed by the M0 config except `seq_level_idx_0`, which depends on
/// the image size.
#[derive(Debug, Clone, Copy)]
pub struct Av1StillConfig {
    /// `seq_profile` = 1 (High; required for 8-bit 4:4:4).
    pub seq_profile: u8,
    /// `seq_level_idx[0]`, the smallest level (≤ 6.0) whose limits cover the image.
    pub seq_level_idx_0: u8,
    /// `seq_tier[0]` = 0.
    pub seq_tier_0: u8,
    /// `high_bitdepth` = false (8-bit).
    pub high_bitdepth: bool,
    /// `twelve_bit` = false.
    pub twelve_bit: bool,
    /// `mono_chrome` = false.
    pub monochrome: bool,
    /// `subsampling_x` = 0 (4:4:4).
    pub chroma_subsampling_x: u8,
    /// `subsampling_y` = 0 (4:4:4).
    pub chroma_subsampling_y: u8,
    /// `chroma_sample_position` = 0.
    pub chroma_sample_position: u8,
    /// CICP `color_primaries` (BT.709 = 1).
    pub color_primaries: u16,
    /// CICP `transfer_characteristics` (sRGB = 13).
    pub transfer_characteristics: u16,
    /// CICP `matrix_coefficients` (Identity = 0).
    pub matrix_coefficients: u16,
    /// `color_range` (full = true).
    pub full_range: bool,
}

/// Level limits (`seq_level_idx`, MaxHSize, MaxVSize, MaxPicSize) for the defined levels up to 6.0
/// (AV1 Annex A §10.3). One representative entry per spatial tier — sufficient to choose the
/// smallest covering level.
const LEVELS: [(u8, u32, u32, u64); 7] = [
    (0, 2048, 1152, 147_456),      // 2.0
    (1, 2816, 1584, 278_784),      // 2.1
    (4, 4352, 2448, 665_856),      // 3.0
    (5, 5504, 3096, 1_065_024),    // 3.1
    (8, 6144, 3456, 2_359_296),    // 4.0
    (12, 8192, 4352, 8_912_896),   // 5.0
    (16, 16384, 8704, 35_651_584), // 6.0
];

/// Returns the smallest `seq_level_idx` (≤ 6.0) whose limits cover `width × height`.
///
/// # Errors
///
/// [`Error::Unsupported`] if the image exceeds the level-6.0 limits (out of M0 scope).
pub fn pick_level(width: u32, height: u32) -> Result<u8> {
    let pixels = u64::from(width) * u64::from(height);
    for &(idx, max_h, max_v, max_pic) in &LEVELS {
        if width <= max_h && height <= max_v && pixels <= max_pic {
            return Ok(idx);
        }
    }
    Err(Error::Unsupported(
        "image dimensions exceed AV1 level 6.0 limits",
    ))
}

/// Number of bits needed to hold `value - 1` (the `frame_width/height_bits` field), minimum 1.
fn dimension_bits(value: u32) -> u32 {
    (32 - value.saturating_sub(1).leading_zeros()).max(1)
}

/// `tile_log2(blkSize, target)` (AV1 §5.9.16): smallest `k` with `blkSize << k >= target`.
fn tile_log2(blk_size: u32, target: u32) -> u32 {
    let mut k = 0;
    while (blk_size << k) < target {
        k += 1;
    }
    k
}

/// Appends an OBU (header byte, `obu_has_size_field = 1`, LEB128 size, payload) to `out`.
pub(crate) fn write_obu(out: &mut Vec<u8>, obu_type: u8, payload: &[u8]) {
    // obu_forbidden_bit=0, obu_type, obu_extension_flag=0, obu_has_size_field=1, reserved=0.
    out.push((obu_type << 3) | 0b10);
    write_leb128(out, payload.len() as u64);
    out.extend_from_slice(payload);
}

/// Builds the sequence-header OBU payload for the M0 config (reduced still picture, profile 1,
/// 8-bit, identity 4:4:4, full range), terminated with `trailing_bits` (AV1 §5.2, §5.3.4).
/// `lossy` enables `enable_filter_intra` (recursive filter-intra is used only on the lossy path;
/// the lossless path stays DC-only and emits no `use_filter_intra` symbols).
pub(crate) fn sequence_header_payload(
    cfg: &Av1StillConfig,
    width: u32,
    height: u32,
    lossy: bool,
) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.put_bits(u32::from(cfg.seq_profile), 3); // seq_profile
    w.put_bit(1); // still_picture
    w.put_bit(1); // reduced_still_picture_header
    w.put_bits(u32::from(cfg.seq_level_idx_0), 5); // seq_level_idx[0]

    let wbits = dimension_bits(width);
    let hbits = dimension_bits(height);
    w.put_bits(wbits - 1, 4); // frame_width_bits_minus_1
    w.put_bits(hbits - 1, 4); // frame_height_bits_minus_1
    w.put_bits(width - 1, wbits); // max_frame_width_minus_1
    w.put_bits(height - 1, hbits); // max_frame_height_minus_1
    // frame_id_numbers_present_flag = 0 (reduced)
    w.put_bit(0); // use_128x128_superblock = 0
    // Filter-intra (recursive DC-prediction, §7.11.2.3) is used only on the lossy path; the
    // lossless path stays DC-only and emits no `use_filter_intra` symbols.
    w.put_bit(u8::from(lossy)); // enable_filter_intra
    w.put_bit(0); // enable_intra_edge_filter = 0
    w.put_bit(0); // enable_superres = 0
    // CDEF (§7.15) is used only on the lossy path; the lossless path is CodedLossless (CDEF off).
    w.put_bit(u8::from(lossy)); // enable_cdef
    w.put_bit(0); // enable_restoration = 0

    // color_config(): high_bitdepth=0; (profile 1 ⇒ mono_chrome=0 inferred);
    // color_description_present_flag=1; cp/tc/mc; (mc==IDENTITY ⇒ color_range/subsampling
    // inferred, no bits); separate_uv_delta_q=0.
    w.put_bit(0); // high_bitdepth
    w.put_bit(1); // color_description_present_flag
    w.put_bits(u32::from(cfg.color_primaries), 8);
    w.put_bits(u32::from(cfg.transfer_characteristics), 8);
    w.put_bits(u32::from(cfg.matrix_coefficients), 8);
    w.put_bit(0); // separate_uv_delta_q

    w.put_bit(0); // film_grain_params_present

    // trailing_bits: a 1 then zero-pad to byte alignment.
    w.put_bit(1);
    w.byte_align();
    w.into_bytes()
}

/// Builds the uncompressed frame header for the intra keyframe (AV1 §5.9.2), byte-aligned
/// (`byte_alignment`, no trailing one — the tile data follows in the same `OBU_FRAME`).
///
/// `base_q_idx == 0` selects the lossless path (`CodedLossless`, forced `TX_4X4` WHT, in-loop
/// filters/`tx_mode` emit nothing). `base_q_idx > 0` selects the lossy path: `TX_MODE_LARGEST`,
/// in-loop filters present but disabled (all levels 0), and `tx_mode_select = 0`. All quantizer
/// deltas are 0 and `using_qmatrix = 0`.
///
/// The returned bytes precede the tile (symbol-coded) data; together they form the frame OBU
/// payload (AV1 §5.10).
pub(crate) fn frame_header_payload(
    width: u32,
    height: u32,
    mi_cols: u32,
    mi_rows: u32,
    base_q_idx: u8,
) -> Vec<u8> {
    let lossless = base_q_idx == 0;
    let mut w = BitWriter::new();
    // reduced_still_picture_header ⇒ KEY_FRAME, show_frame=1, FrameIsIntra=1 (no bits).
    w.put_bit(1); // disable_cdf_update = 1
    w.put_bit(0); // allow_screen_content_tools = 0
    // force_integer_mv inferred; frame_size_override_flag=0; order_hint f(0); primary_ref_frame
    // inferred; refresh_frame_flags inferred — all no bits.
    // frame_size(): no override ⇒ from seq header; superres disabled. render_size():
    w.put_bit(0); // render_and_frame_size_different = 0
    // disable_frame_end_update_cdf inferred 1.

    // tile_info(): single tile. Emit increment-stop bits only where the level/size permit > 0 tiles.
    let _ = (width, height);
    let sb_cols = (mi_cols + 15) >> 4;
    let sb_rows = (mi_rows + 15) >> 4;
    let max_log2_tile_cols = tile_log2(1, sb_cols.min(64));
    let max_log2_tile_rows = tile_log2(1, sb_rows.min(64));
    w.put_bit(1); // uniform_tile_spacing_flag
    if max_log2_tile_cols > 0 {
        w.put_bit(0); // increment_tile_cols_log2 = 0 (stay at 1 tile column)
    }
    if max_log2_tile_rows > 0 {
        w.put_bit(0); // increment_tile_rows_log2 = 0 (stay at 1 tile row)
    }
    // TileColsLog2 == TileRowsLog2 == 0 ⇒ no context_update_tile_id / tile_size_bytes.

    // quantization_params().
    w.put_bits(u32::from(base_q_idx), 8); // base_q_idx
    w.put_bit(0); // DeltaQYDc: delta_coded = 0
    w.put_bit(0); // DeltaQUDc: delta_coded = 0
    w.put_bit(0); // DeltaQUAc: delta_coded = 0
    w.put_bit(0); // using_qmatrix = 0

    w.put_bit(0); // segmentation_enabled = 0

    if !lossless {
        // delta_q_params(): base_q_idx > 0 ⇒ delta_q_present (0). delta_lf_params(): none.
        w.put_bit(0); // delta_q_present = 0
        // loop_filter_params(): a single deblock level (the same for both luma passes and both
        // chroma planes), scaled from base_q_idx. level 0 ⇒ deblock disabled and level[2]/[3] omitted.
        let lf = u32::from(crate::filter::deblock_level(base_q_idx));
        w.put_bits(lf, 6); // loop_filter_level[0]
        w.put_bits(lf, 6); // loop_filter_level[1]
        if lf != 0 {
            w.put_bits(lf, 6); // loop_filter_level[2] (U)
            w.put_bits(lf, 6); // loop_filter_level[3] (V)
        }
        w.put_bits(0, 3); // loop_filter_sharpness
        w.put_bit(0); // loop_filter_delta_enabled = 0
        // cdef_params(): enable_cdef = 1 ⇒ CdefDamping = 3, cdef_bits = 0 (one strength set applied
        // everywhere; cdef_idx is then L(0) = 0 bits in the tile). The secondary strength is signaled
        // pre-mapping (the decoder maps 3 → 4), so a stored 4 is emitted as 3.
        let (y_pri, y_sec, uv_pri, uv_sec) = crate::filter::cdef_strengths(base_q_idx);
        let sec_code = |s: i32| -> u32 { if s == 4 { 3 } else { s as u32 } };
        w.put_bits(0, 2); // cdef_damping_minus_3 (CdefDamping = 3)
        w.put_bits(0, 2); // cdef_bits = 0
        w.put_bits(y_pri as u32, 4); // cdef_y_pri_strength[0]
        w.put_bits(sec_code(y_sec), 2); // cdef_y_sec_strength[0]
        w.put_bits(uv_pri as u32, 4); // cdef_uv_pri_strength[0]
        w.put_bits(sec_code(uv_sec), 2); // cdef_uv_sec_strength[0]
        // lr_params(): enable_restoration = 0 ⇒ no bits.
        w.put_bit(0); // read_tx_mode: tx_mode_select = 0 ⇒ TX_MODE_LARGEST
        // frame_reference_mode / skip_mode_params (intra) ⇒ no bits. allow_warped_motion = 0.
    }
    // CodedLossless ⇒ loop_filter / cdef / lr / tx_mode emit nothing (TxMode = ONLY_4X4).
    w.put_bit(1); // reduced_tx_set = 1
    // global_motion_params / film_grain_params (intra, not present) ⇒ no bits.

    w.byte_align();
    w.into_bytes()
}

/// Wraps the sequence-header and frame OBUs into the temporal unit placed in `mdat`.
pub(crate) fn assemble_temporal_unit(seq_payload: &[u8], frame_payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    write_obu(&mut out, OBU_SEQUENCE_HEADER, seq_payload);
    write_obu(&mut out, OBU_FRAME, frame_payload);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_selection() {
        assert_eq!(pick_level(16, 16).unwrap(), 0);
        // 1920x1080 = 2_073_600 px exceeds 3.x MaxPicSize, so level 4.0 (idx 8) is smallest.
        assert_eq!(pick_level(1920, 1080).unwrap(), 8);
        assert!(pick_level(100_000, 100_000).is_err());
    }

    #[test]
    fn dimension_bits_min_one() {
        assert_eq!(dimension_bits(1), 1); // W-1 = 0
        assert_eq!(dimension_bits(16), 4); // 15 -> 4 bits
        assert_eq!(dimension_bits(17), 5); // 16 -> 5 bits
        assert_eq!(dimension_bits(256), 8); // 255 -> 8 bits
        assert_eq!(dimension_bits(257), 9); // 256 -> 9 bits
    }

    #[test]
    fn obu_framing_has_size_field() {
        let mut out = Vec::new();
        write_obu(&mut out, OBU_SEQUENCE_HEADER, &[0xaa, 0xbb]);
        // header byte: type 1 -> (1<<3)|2 = 0x0a; size leb128 = 2; payload.
        assert_eq!(out, vec![0x0a, 0x02, 0xaa, 0xbb]);
    }
}
