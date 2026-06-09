//! VP8L bitstream header: signature, dimensions, and feature hints (RFC 9649 §3.4).

/// The byte that begins every VP8L bitstream (RFC 9649 §3.4).
pub const VP8L_SIGNATURE: u8 = 0x2f;

/// Maximum VP8L image dimension (the 14-bit width/height fields encode `1..=16384`).
pub const VP8L_MAX_DIMENSION: u16 = 1 << 14;

/// A decoded VP8L bitstream header (RFC 9649 §3.4).
///
/// On the wire the [`VP8L_SIGNATURE`] byte is followed by 14-bit `width - 1` and `height - 1`
/// fields, a 1-bit alpha hint, and a 3-bit version (the only defined value is 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Vp8lHeader {
    /// Image width in pixels (`1..=16384`; stored on the wire as `width - 1`).
    pub width: u16,
    /// Image height in pixels (`1..=16384`; stored on the wire as `height - 1`).
    pub height: u16,
    /// The `alpha_is_used` hint: whether any pixel has a non-opaque alpha value.
    pub alpha_is_used: bool,
    /// Bitstream version number; the only defined value is 0.
    pub version: u8,
}
