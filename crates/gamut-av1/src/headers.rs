//! AV1 OBU framing (§5.3, Annex B), the reduced-still-picture sequence header (§5.5), and the
//! lossless intra-keyframe uncompressed frame header (§5.9.2), specialised to the M0 config.

use gamut_bitstream::{BitWriter, write_leb128};
use gamut_core::{Error, Result};

/// `OBU_SEQUENCE_HEADER`.
const OBU_SEQUENCE_HEADER: u8 = 1;
/// `OBU_FRAME` (frame header + tile group in one OBU).
pub(crate) const OBU_FRAME: u8 = 6;

/// `TileSizeBytes` (§5.11.1): the width of each non-final tile's little-endian size prefix in the
/// tile group. Four bytes covers any tile a still image produces; signalled as `minus_1` in the
/// frame header's `tile_info`.
pub(crate) const TILE_SIZE_BYTES: usize = 4;

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
    superres: bool,
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
    w.put_bit(u8::from(superres)); // enable_superres
    // CDEF (§7.15) is used only on the lossy path; the lossless path is CodedLossless (CDEF off).
    w.put_bit(u8::from(lossy)); // enable_cdef
    w.put_bit(u8::from(lossy)); // enable_restoration (1 on the lossy path: luma Wiener)

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
/// filters/`tx_mode` emit nothing). `base_q_idx > 0` selects the lossy path: `TX_MODE_SELECT`
/// (per-block `tx_depth`), in-loop filters present but disabled (all levels 0). All quantizer
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
    superres_coded_denom: Option<u8>,
) -> Vec<u8> {
    let lossless = base_q_idx == 0;
    let mut w = BitWriter::new();
    // reduced_still_picture_header ⇒ KEY_FRAME, show_frame=1, FrameIsIntra=1 (no bits).
    w.put_bit(1); // disable_cdf_update = 1
    // allow_screen_content_tools = 1 (palette mode is available; intra-block-copy is left off).
    w.put_bit(u8::from(!lossless)); // allow_screen_content_tools (1 on the lossy path)
    if !lossless {
        // seq_force_integer_mv == SELECT_INTEGER_MV (reduced still picture) ⇒ force_integer_mv is
        // coded when screen-content tools are on. The value is irrelevant (an intra frame overrides
        // it to 1), so 0 is emitted.
        w.put_bit(0); // force_integer_mv
    }
    // frame_size_override_flag=0 (reduced_still_picture_header); order_hint f(0); primary_ref_frame
    // inferred; refresh_frame_flags inferred — all no bits.
    // frame_size(): no override ⇒ UpscaledWidth from the sequence header. superres_params (§5.9.8) is
    // present only when enable_superres; then FrameWidth = downscaled and the reconstruction is
    // upscaled back to UpscaledWidth.
    if let Some(cd) = superres_coded_denom {
        w.put_bit(1); // use_superres
        w.put_bits(u32::from(cd), 3); // coded_denom (SuperresDenom = coded_denom + 9)
    }
    // render_size():
    w.put_bit(0); // render_and_frame_size_different = 0
    if !lossless && superres_coded_denom.is_none() {
        // allow_intrabc is coded only when UpscaledWidth == FrameWidth (i.e. no superres).
        w.put_bit(0); // allow_intrabc = 0
    }
    // disable_frame_end_update_cdf inferred 1.

    // tile_info() (§5.9.15): uniform spacing, two tile columns when the frame is ≥ 2 superblocks
    // wide (else one). `tile_cols_log2` mirrors the split `FrameEncoder::encode` applies.
    let _ = (width, height);
    let sb_cols = (mi_cols + 15) >> 4;
    let sb_rows = (mi_rows + 15) >> 4;
    let max_log2_tile_cols = tile_log2(1, sb_cols.min(64));
    let max_log2_tile_rows = tile_log2(1, sb_rows.min(64));
    let tile_cols_log2 = u32::from(sb_cols >= 2);
    w.put_bit(1); // uniform_tile_spacing_flag
    // increment_tile_cols_log2 loop (min log2 is 0 at these sizes): 1 to grow TileColsLog2, 0 to stop.
    let mut t = 0;
    while t < max_log2_tile_cols {
        let inc = t < tile_cols_log2;
        w.put_bit(u8::from(inc));
        if inc {
            t += 1;
        } else {
            break;
        }
    }
    if max_log2_tile_rows > 0 {
        w.put_bit(0); // increment_tile_rows_log2 = 0 (one tile row)
    }
    if tile_cols_log2 > 0 {
        // context_update_tile_id is `TileRowsLog2 + TileColsLog2` bits (tile 0); then
        // tile_size_bytes_minus_1 (2 bits).
        w.put_bits(0, tile_cols_log2);
        w.put_bits(TILE_SIZE_BYTES as u32 - 1, 2);
    }

    // quantization_params().
    w.put_bits(u32::from(base_q_idx), 8); // base_q_idx
    w.put_bit(0); // DeltaQYDc: delta_coded = 0
    w.put_bit(0); // DeltaQUDc: delta_coded = 0
    w.put_bit(0); // DeltaQUAc: delta_coded = 0
    w.put_bit(0); // using_qmatrix = 0

    // segmentation_params(): on the lossy path, enable per-segment alternate quantizers
    // (SEG_LVL_ALT_Q) on a couple of segments so blocks can carry their own quantizer.
    if lossless {
        w.put_bit(0); // segmentation_enabled = 0
    } else {
        w.put_bit(1); // segmentation_enabled = 1
        // primary_ref_frame == PRIMARY_REF_NONE ⇒ update_map/temporal/data are inferred (no bits).
        // feature loop: MAX_SEGMENTS (8) × SEG_LVL_MAX (8). Only SEG_LVL_ALT_Q (feature 0) is used.
        for seg in 0..8usize {
            for feat in 0..8usize {
                if let (0, Some(delta)) = (feat, crate::tile::SEG_ALT_Q[seg]) {
                    w.put_bit(1); // feature_enabled = 1
                    // feature_value su(1 + Segmentation_Feature_Bits[ALT_Q]=8) = su(9), signed.
                    w.put_bits((delta & 0x1FF) as u32, 9);
                    continue;
                }
                w.put_bit(0); // feature_enabled = 0
            }
        }
    }

    if !lossless {
        // delta_q_params(): base_q_idx > 0 ⇒ delta_q_present = 1, delta_q_res = 0 (deltas in qindex
        // units). delta_lf_params(): delta_lf_present = 0 (the loop-filter level stays frame-level).
        w.put_bit(1); // delta_q_present = 1
        w.put_bits(0, 2); // delta_q_res = 0
        // delta_lf_params(): per-superblock loop-filter-level deltas (single, not multi).
        w.put_bit(1); // delta_lf_present = 1
        w.put_bits(0, 2); // delta_lf_res = 0
        w.put_bit(0); // delta_lf_multi = 0
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
        // lr_params() (§5.9.20): luma RESTORE_WIENER, chroma RESTORE_NONE. `lr_type` is 2 bits per
        // plane (`Remap_Lr_Type`: 0=NONE, 2=WIENER); only luma uses restoration.
        w.put_bits(2, 2); // FrameRestorationType[0] = RESTORE_WIENER
        w.put_bits(0, 2); // FrameRestorationType[1] = RESTORE_NONE
        w.put_bits(0, 2); // FrameRestorationType[2] = RESTORE_NONE
        // usesLr ⇒ lr_unit_shift. Not a 128×128 superblock ⇒ f(1) then (if set) lr_unit_extra f(1).
        // shift = 2 ⇒ LoopRestorationSize = 256. usesChromaLr = 0 ⇒ no lr_uv_shift.
        w.put_bit(1); // lr_unit_shift bit 0
        w.put_bit(1); // lr_unit_extra ⇒ lr_unit_shift = 2
        w.put_bit(1); // read_tx_mode: tx_mode_select = 1 ⇒ TX_MODE_SELECT (per-block tx_depth)
        // frame_reference_mode / skip_mode_params (intra) ⇒ no bits. allow_warped_motion = 0.
    }
    // CodedLossless ⇒ loop_filter / cdef / lr / tx_mode emit nothing (TxMode = ONLY_4X4).
    w.put_bit(1); // reduced_tx_set = 1
    // global_motion_params / film_grain_params (intra, not present) ⇒ no bits.

    w.byte_align(); // byte_alignment after frame_header_obu (§5.10)
    // tile_group_obu prefix (§5.11.1): for more than one tile, tile_start_and_end_present_flag = 0,
    // then re-align so the tile_size fields / tile data begin on a byte boundary.
    if tile_cols_log2 > 0 {
        w.put_bit(0); // tile_start_and_end_present_flag
        w.byte_align();
    }
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
