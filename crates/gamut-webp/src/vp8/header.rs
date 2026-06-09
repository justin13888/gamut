//! VP8 key-frame frame header (RFC 6386 §9): the uncompressed 10-byte chunk plus the
//! boolean-coded header fields (segmentation, loop filter, partitions, quantization).

/// The 3-byte start code that follows the 1-byte frame tag in a VP8 key frame (RFC 6386 §9.1).
pub const VP8_KEYFRAME_START_CODE: [u8; 3] = [0x9d, 0x01, 0x2a];

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

/// A decoded VP8 key-frame header (RFC 6386 §9). Intra/key-frame fields only — gamut codes no
/// inter-frame state.
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
}
