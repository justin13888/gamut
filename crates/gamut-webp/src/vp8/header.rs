//! VP8 key-frame frame header (RFC 6386 §9, §19.1–§19.2): the uncompressed 10-byte chunk (frame tag,
//! start code, dimensions) plus the boolean-coded header fields (color space, loop filter, partition
//! count, quantizer indices, coefficient-probability updates).
//!
//! gamut codes key frames only (`key_frame` bit = 0). The encoder emits the minimal header a
//! still image needs — segmentation and per-macroblock loop-filter adjustments disabled, no
//! coefficient-probability updates — while the decoder fully parses the quantizer indices and the
//! token-probability-update record (so it tracks the working [`CoeffProbs`]); the segmentation and
//! loop-filter-adjustment *bodies* are rejected for now and land with their codec features (P11/P12).
//! Tracked in `../STATUS.md` section H.

use gamut_core::{Error, Result};

use super::bool_coder::{BoolDecoder, BoolEncoder};
use super::tokens::{self, CoeffProbs, DEFAULT_COEFF_PROBS};

/// The 3-byte start code that follows the frame tag in a VP8 key frame (RFC 6386 §9.1).
pub const VP8_KEYFRAME_START_CODE: [u8; 3] = [0x9d, 0x01, 0x2a];

/// Length in bytes of a key-frame's uncompressed data chunk (RFC 6386 §9.1).
pub const UNCOMPRESSED_CHUNK_LEN: usize = 10;

/// Per-segment adjustment state (RFC 6386 §9.3, §10). Still images usually leave this disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Segmentation {
    /// Whether segmentation is enabled for the frame.
    pub enabled: bool,
    /// Whether the per-macroblock segment map is (re)transmitted this frame.
    pub update_map: bool,
    /// Per-segment quantizer adjustment (absolute or delta, per the frame's `abs_delta` flag).
    pub quantizer: [i8; 4],
    /// Per-segment loop-filter-level adjustment.
    pub filter_strength: [i8; 4],
}

/// Loop-filter header parameters (RFC 6386 §9.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LoopFilterParams {
    /// `true` selects the simple filter; `false` selects the normal filter.
    pub simple: bool,
    /// Base filter level (`0..=63`); 0 disables the loop filter.
    pub level: u8,
    /// Sharpness level (`0..=7`).
    pub sharpness: u8,
}

/// Dequantization indices (RFC 6386 §9.6): a base AC index plus a signed delta per plane/coefficient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QuantIndices {
    /// Base quantizer index (the Y1 AC index, `0..=127`).
    pub y_ac: u8,
    /// Y1 DC index delta.
    pub y_dc_delta: i8,
    /// Y2 (WHT) DC index delta.
    pub y2_dc_delta: i8,
    /// Y2 (WHT) AC index delta.
    pub y2_ac_delta: i8,
    /// Chroma DC index delta.
    pub uv_dc_delta: i8,
    /// Chroma AC index delta.
    pub uv_ac_delta: i8,
}

/// A VP8 key-frame header (RFC 6386 §9). Intra/key-frame fields only — gamut codes no inter-frame
/// state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vp8FrameHeader {
    /// Frame width in pixels (the 14-bit field of the uncompressed chunk).
    pub width: u16,
    /// Frame height in pixels (the 14-bit field of the uncompressed chunk).
    pub height: u16,
    /// Horizontal upscaling hint (2 bits; 0 = none).
    pub horizontal_scale: u8,
    /// Vertical upscaling hint (2 bits; 0 = none).
    pub vertical_scale: u8,
    /// Bitstream version (3 bits; selects loop-filter / reconstruction variants).
    pub version: u8,
    /// Color space (0 = YUV per BT.601; 1 is reserved).
    pub color_space: u8,
    /// Whether pixel clamping is required (the `clamping_type` flag).
    pub clamp_required: bool,
    /// Segmentation state (§9.3).
    pub segmentation: Segmentation,
    /// Loop-filter header (§9.4).
    pub loop_filter: LoopFilterParams,
    /// Number of DCT-coefficient token partitions (1, 2, 4, or 8) (§9.5).
    pub token_partitions: u8,
    /// Dequantization indices (§9.6).
    pub quant: QuantIndices,
    /// Whether token-probability updates persist past this frame (§9.11).
    pub refresh_entropy_probs: bool,
    /// Whether macroblocks may signal that they carry no non-zero coefficients (§9.10).
    pub mb_no_skip_coeff: bool,
    /// Probability that a macroblock is *not* skipped (only meaningful if `mb_no_skip_coeff`) (§9.10).
    pub prob_skip_false: u8,
}

/// The parsed uncompressed data chunk (RFC 6386 §9.1, §19.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UncompressedChunk {
    /// Whether this is a key frame (the frame-tag bit is `0` for key frames).
    pub is_key_frame: bool,
    /// Bitstream version (3 bits).
    pub version: u8,
    /// Whether the frame is meant to be displayed.
    pub show_frame: bool,
    /// Size in bytes of the first (control) partition, excluding this chunk.
    pub first_partition_size: u32,
    /// Frame width in pixels (14 bits).
    pub width: u16,
    /// Frame height in pixels (14 bits).
    pub height: u16,
    /// Horizontal upscaling hint (2 bits).
    pub horizontal_scale: u8,
    /// Vertical upscaling hint (2 bits).
    pub vertical_scale: u8,
}

/// `log2` of a token-partition count `{1, 2, 4, 8}`.
fn log2_partitions(count: u8) -> u32 {
    debug_assert!(
        matches!(count, 1 | 2 | 4 | 8),
        "token partition count must be 1, 2, 4, or 8"
    );
    u32::from(count).trailing_zeros()
}

/// Writes the 10-byte uncompressed data chunk for a key frame (RFC 6386 §19.1) to `out`:
/// the frame tag (with `first_partition_size` and `show_frame = 1`), the start code, and the
/// little-endian width/height + scale codes.
pub fn write_uncompressed_chunk(
    header: &Vp8FrameHeader,
    first_partition_size: u32,
    out: &mut Vec<u8>,
) {
    // key_frame bit (bit 0) = 0; version in bits 1-3; show_frame = 1 in bit 4; size in bits 5-23.
    let tag = (u32::from(header.version) << 1) | (1 << 4) | (first_partition_size << 5);
    out.push((tag & 0xff) as u8);
    out.push(((tag >> 8) & 0xff) as u8);
    out.push(((tag >> 16) & 0xff) as u8);
    out.extend_from_slice(&VP8_KEYFRAME_START_CODE);
    let h = u32::from(header.width) | (u32::from(header.horizontal_scale) << 14);
    out.push((h & 0xff) as u8);
    out.push(((h >> 8) & 0xff) as u8);
    let v = u32::from(header.height) | (u32::from(header.vertical_scale) << 14);
    out.push((v & 0xff) as u8);
    out.push(((v >> 8) & 0xff) as u8);
}

/// Parses the uncompressed data chunk (RFC 6386 §19.1).
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if the data is too short or the key-frame start code is wrong, or
/// [`Error::Unsupported`] for an inter frame (gamut codes key frames only).
pub fn read_uncompressed_chunk(data: &[u8]) -> Result<UncompressedChunk> {
    if data.len() < 3 {
        return Err(Error::InvalidInput("VP8: truncated frame tag"));
    }
    let tag = u32::from(data[0]) | (u32::from(data[1]) << 8) | (u32::from(data[2]) << 16);
    let is_key_frame = (tag & 1) == 0;
    if !is_key_frame {
        return Err(Error::Unsupported(
            "VP8: only intra key frames are supported",
        ));
    }
    if data.len() < UNCOMPRESSED_CHUNK_LEN {
        return Err(Error::InvalidInput("VP8: truncated key-frame header"));
    }
    if data[3..6] != VP8_KEYFRAME_START_CODE {
        return Err(Error::InvalidInput("VP8: bad key-frame start code"));
    }
    let hsc = u32::from(data[6]) | (u32::from(data[7]) << 8);
    let vsc = u32::from(data[8]) | (u32::from(data[9]) << 8);
    Ok(UncompressedChunk {
        is_key_frame,
        version: ((tag >> 1) & 0x7) as u8,
        show_frame: (tag >> 4) & 1 != 0,
        first_partition_size: (tag >> 5) & 0x7_FFFF,
        width: (hsc & 0x3FFF) as u16,
        horizontal_scale: (hsc >> 14) as u8,
        height: (vsc & 0x3FFF) as u16,
        vertical_scale: (vsc >> 14) as u8,
    })
}

/// Writes a signed quantizer-index delta as `present` flag + magnitude `L(4)` + sign (RFC 6386 §19.2).
fn write_delta(enc: &mut BoolEncoder, delta: i8) {
    if delta == 0 {
        enc.put_flag(false);
    } else {
        enc.put_flag(true);
        enc.put_literal(u32::from(delta.unsigned_abs()), 4);
        enc.put_flag(delta < 0);
    }
}

/// Reads a signed quantizer-index delta (RFC 6386 §19.2).
fn read_delta(dec: &mut BoolDecoder) -> i8 {
    if dec.get_flag() {
        let magnitude = dec.get_literal(4) as i8;
        if dec.get_flag() {
            -magnitude
        } else {
            magnitude
        }
    } else {
        0
    }
}

/// Writes `quant_indices()` (RFC 6386 §19.2): the base AC index then the five per-plane deltas.
fn write_quant_indices(enc: &mut BoolEncoder, quant: &QuantIndices) {
    enc.put_literal(u32::from(quant.y_ac), 7);
    write_delta(enc, quant.y_dc_delta);
    write_delta(enc, quant.y2_dc_delta);
    write_delta(enc, quant.y2_ac_delta);
    write_delta(enc, quant.uv_dc_delta);
    write_delta(enc, quant.uv_ac_delta);
}

/// Reads `quant_indices()` (RFC 6386 §19.2).
fn read_quant_indices(dec: &mut BoolDecoder) -> QuantIndices {
    QuantIndices {
        y_ac: dec.get_literal(7) as u8,
        y_dc_delta: read_delta(dec),
        y2_dc_delta: read_delta(dec),
        y2_ac_delta: read_delta(dec),
        uv_dc_delta: read_delta(dec),
        uv_ac_delta: read_delta(dec),
    }
}

/// Writes the boolean-coded key-frame header (RFC 6386 §19.2) into the first (control) partition's
/// encoder `enc`, leaving it open for the per-macroblock records that follow. Emits the minimal
/// header: segmentation and loop-filter adjustments disabled, no coefficient-probability updates.
pub fn write_frame_header(enc: &mut BoolEncoder, header: &Vp8FrameHeader) {
    debug_assert!(
        !header.segmentation.enabled,
        "P6 encodes segmentation-disabled headers only"
    );
    enc.put_literal(u32::from(header.color_space), 1);
    enc.put_flag(!header.clamp_required); // clamping_type: 1 = no clamp needed
    enc.put_flag(false); // segmentation_enabled (body in P12)
    enc.put_flag(header.loop_filter.simple); // filter_type
    enc.put_literal(u32::from(header.loop_filter.level), 6);
    enc.put_literal(u32::from(header.loop_filter.sharpness), 3);
    enc.put_flag(false); // loop_filter_adj_enable (body in P11)
    enc.put_literal(log2_partitions(header.token_partitions), 2);
    write_quant_indices(enc, &header.quant);
    enc.put_flag(header.refresh_entropy_probs);
    tokens::write_coeff_prob_updates(enc, &DEFAULT_COEFF_PROBS, &DEFAULT_COEFF_PROBS);
    enc.put_flag(header.mb_no_skip_coeff);
    if header.mb_no_skip_coeff {
        enc.put_literal(u32::from(header.prob_skip_false), 8);
    }
}

/// Reads the boolean-coded key-frame header (RFC 6386 §19.2) from the control-partition decoder `dec`,
/// returning the header and the working coefficient-probability table after any updates.
///
/// # Errors
///
/// Returns [`Error::Unsupported`] for segmentation or loop-filter adjustments (those land with their
/// codec features in P11/P12).
pub fn read_frame_header(
    chunk: &UncompressedChunk,
    dec: &mut BoolDecoder,
) -> Result<(Vp8FrameHeader, CoeffProbs)> {
    let color_space = dec.get_literal(1) as u8;
    let clamp_required = !dec.get_flag();
    if dec.get_flag() {
        return Err(Error::Unsupported("VP8: segmentation not yet supported"));
    }
    let loop_filter = LoopFilterParams {
        simple: dec.get_flag(),
        level: dec.get_literal(6) as u8,
        sharpness: dec.get_literal(3) as u8,
    };
    if dec.get_flag() {
        return Err(Error::Unsupported(
            "VP8: loop-filter adjustments not yet supported",
        ));
    }
    let token_partitions = 1u8 << dec.get_literal(2);
    let quant = read_quant_indices(dec);
    let refresh_entropy_probs = dec.get_flag();
    let mut coeff_probs = DEFAULT_COEFF_PROBS;
    tokens::read_coeff_prob_updates(dec, &mut coeff_probs);
    let mb_no_skip_coeff = dec.get_flag();
    let prob_skip_false = if mb_no_skip_coeff {
        dec.get_literal(8) as u8
    } else {
        0
    };
    let header = Vp8FrameHeader {
        width: chunk.width,
        height: chunk.height,
        horizontal_scale: chunk.horizontal_scale,
        vertical_scale: chunk.vertical_scale,
        version: chunk.version,
        color_space,
        clamp_required,
        segmentation: Segmentation::default(),
        loop_filter,
        token_partitions,
        quant,
        refresh_entropy_probs,
        mb_no_skip_coeff,
        prob_skip_false,
    };
    Ok((header, coeff_probs))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_header() -> Vp8FrameHeader {
        Vp8FrameHeader {
            width: 176,
            height: 144,
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
            quant: QuantIndices::default(),
            refresh_entropy_probs: true,
            mb_no_skip_coeff: false,
            prob_skip_false: 0,
        }
    }

    /// Encodes a header to a complete (header-only) VP8 bitstream and decodes it back.
    fn roundtrip(header: &Vp8FrameHeader) {
        let mut enc = BoolEncoder::new();
        write_frame_header(&mut enc, header);
        let part0 = enc.finish();
        let mut stream = Vec::new();
        write_uncompressed_chunk(header, part0.len() as u32, &mut stream);
        stream.extend_from_slice(&part0);

        let chunk = read_uncompressed_chunk(&stream).expect("chunk");
        assert!(chunk.is_key_frame);
        assert_eq!((chunk.width, chunk.height), (header.width, header.height));
        assert_eq!(chunk.first_partition_size as usize, part0.len());

        let end = UNCOMPRESSED_CHUNK_LEN + chunk.first_partition_size as usize;
        let mut dec = BoolDecoder::new(&stream[UNCOMPRESSED_CHUNK_LEN..end]);
        let (decoded, probs) = read_frame_header(&chunk, &mut dec).expect("header");
        assert_eq!(&decoded, header);
        assert_eq!(
            probs, DEFAULT_COEFF_PROBS,
            "minimal header carries no prob updates"
        );
    }

    #[test]
    fn minimal_header_round_trips() {
        roundtrip(&sample_header());
    }

    #[test]
    fn dimensions_and_scale_round_trip() {
        for (w, h, hs, vs) in [
            (1u16, 1u16, 0u8, 0u8),
            (16, 16, 0, 0),
            (16383, 1, 3, 0),
            (17, 9, 1, 2),
        ] {
            let mut header = sample_header();
            header.width = w;
            header.height = h;
            header.horizontal_scale = hs;
            header.vertical_scale = vs;
            roundtrip(&header);
        }
    }

    #[test]
    fn quant_filter_and_flags_round_trip() {
        let mut header = sample_header();
        header.quant = QuantIndices {
            y_ac: 100,
            y_dc_delta: 7,
            y2_dc_delta: -8,
            y2_ac_delta: 15,
            uv_dc_delta: -1,
            uv_ac_delta: 0,
        };
        header.loop_filter = LoopFilterParams {
            simple: true,
            level: 47,
            sharpness: 5,
        };
        header.color_space = 1;
        header.clamp_required = false;
        header.refresh_entropy_probs = false;
        header.version = 3;
        roundtrip(&header);
    }

    #[test]
    fn skip_probability_round_trips() {
        let mut header = sample_header();
        header.mb_no_skip_coeff = true;
        header.prob_skip_false = 210;
        roundtrip(&header);
    }

    #[test]
    fn partition_counts_round_trip() {
        for count in [1u8, 2, 4, 8] {
            let mut header = sample_header();
            header.token_partitions = count;
            roundtrip(&header);
        }
    }

    #[test]
    fn rejects_inter_frame_and_bad_start_code() {
        // Inter frame: frame-tag bit 0 set.
        assert!(matches!(
            read_uncompressed_chunk(&[0x01, 0, 0, 0x9d, 0x01, 0x2a, 0, 0, 0, 0]),
            Err(Error::Unsupported(_))
        ));
        // Key frame with a corrupted start code.
        assert!(matches!(
            read_uncompressed_chunk(&[0x00, 0, 0, 0x9d, 0x01, 0x2b, 16, 0, 16, 0]),
            Err(Error::InvalidInput(_))
        ));
        // Truncated.
        assert!(read_uncompressed_chunk(&[0x00, 0, 0]).is_err());
    }

    #[test]
    fn rejects_unsupported_segmentation_and_lf_adjust() {
        let chunk = UncompressedChunk {
            is_key_frame: true,
            version: 0,
            show_frame: true,
            first_partition_size: 0,
            width: 16,
            height: 16,
            horizontal_scale: 0,
            vertical_scale: 0,
        };
        // color_space, clamping, segmentation_enabled = 1.
        let mut seg = BoolEncoder::new();
        seg.put_literal(0, 1);
        seg.put_flag(true);
        seg.put_flag(true);
        let bytes = seg.finish();
        assert!(matches!(
            read_frame_header(&chunk, &mut BoolDecoder::new(&bytes)),
            Err(Error::Unsupported(_))
        ));

        // color_space, clamping, segmentation=0, filter_type, level(6), sharpness(3), lf_adj = 1.
        let mut lf = BoolEncoder::new();
        lf.put_literal(0, 1);
        lf.put_flag(true);
        lf.put_flag(false);
        lf.put_flag(false);
        lf.put_literal(0, 6);
        lf.put_literal(0, 3);
        lf.put_flag(true);
        let bytes = lf.finish();
        assert!(matches!(
            read_frame_header(&chunk, &mut BoolDecoder::new(&bytes)),
            Err(Error::Unsupported(_))
        ));
    }
}
