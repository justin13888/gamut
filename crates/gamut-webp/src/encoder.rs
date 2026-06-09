//! The public WebP encoder: orchestrates color handling, the VP8/VP8L bitstream, and the RIFF
//! container, mirroring the shape of [`gamut_avif::AvifEncoder`](https://docs.rs/gamut-avif).
//!
//! The bitstream paths are under construction (tracked in `../STATUS.md`), so [`WebpEncoder`]
//! validates its inputs and then returns [`Error::Unsupported`] until the VP8L M0 path lands.

use gamut_core::{Dimensions, Encoder, Error, Result};

use crate::config::{WebpConfig, WebpMode};

/// Encodes 8-bit RGB images to WebP.
///
/// Construct with [`WebpEncoder::new`] (lossless), [`WebpEncoder::lossless`], or
/// [`WebpEncoder::lossy`], then call [`WebpEncoder::encode_rgb8`] (or the [`Encoder`] trait method).
#[derive(Debug, Clone, Default)]
pub struct WebpEncoder {
    /// Encoder configuration (mode + quality).
    config: WebpConfig,
}

impl WebpEncoder {
    /// Creates an encoder with the default configuration (lossless VP8L).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an encoder that produces a lossless VP8L bitstream.
    #[must_use]
    pub fn lossless() -> Self {
        Self {
            config: WebpConfig {
                mode: WebpMode::Lossless,
                ..WebpConfig::default()
            },
        }
    }

    /// Creates an encoder that produces a lossy VP8 bitstream at the given `quality` (`0..=100`).
    #[must_use]
    pub fn lossy(quality: u8) -> Self {
        Self {
            config: WebpConfig {
                mode: WebpMode::Lossy,
                quality,
            },
        }
    }

    /// Returns the encoder's configuration.
    #[must_use]
    pub fn config(&self) -> WebpConfig {
        self.config
    }

    /// Encodes interleaved 8-bit RGB `pixels` (row-major, 3 bytes per pixel) described by `dims`,
    /// appending the WebP file to `out` and returning the number of bytes written.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `pixels.len()` does not equal `width * height * 3`, or
    /// [`Error::Unsupported`] for the requested mode until its bitstream path is implemented.
    pub fn encode_rgb8(&self, pixels: &[u8], dims: Dimensions, out: &mut Vec<u8>) -> Result<usize> {
        let expected = (dims.width as usize)
            .checked_mul(dims.height as usize)
            .and_then(|n| n.checked_mul(3));
        if expected != Some(pixels.len()) {
            return Err(Error::InvalidInput(
                "WebP: RGB buffer length does not match dimensions",
            ));
        }
        // The container/color plumbing is ready (see gamut-riff); the bitstream encoders are not.
        let _ = out;
        match self.config.mode {
            WebpMode::Lossless => Err(Error::Unsupported(
                "WebP VP8L (lossless) encoding not yet implemented",
            )),
            WebpMode::Lossy => Err(Error::Unsupported(
                "WebP VP8 (lossy) encoding not yet implemented",
            )),
        }
    }
}

impl Encoder for WebpEncoder {
    fn encode(&self, pixels: &[u8], dims: Dimensions, out: &mut Vec<u8>) -> Result<usize> {
        self.encode_rgb8(pixels, dims, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dims(w: u32, h: u32) -> Dimensions {
        Dimensions {
            width: w,
            height: h,
        }
    }

    #[test]
    fn constructors_select_mode() {
        assert_eq!(WebpEncoder::new().config().mode, WebpMode::Lossless);
        assert_eq!(WebpEncoder::lossless().config().mode, WebpMode::Lossless);
        let lossy = WebpEncoder::lossy(40);
        assert_eq!(lossy.config().mode, WebpMode::Lossy);
        assert_eq!(lossy.config().quality, 40);
    }

    #[test]
    fn rejects_mismatched_buffer_length() {
        let mut out = Vec::new();
        let err = WebpEncoder::new().encode_rgb8(&[0u8; 10], dims(2, 2), &mut out);
        assert!(matches!(err, Err(Error::InvalidInput(_))));
    }

    #[test]
    fn lossless_is_unsupported_for_now() {
        let mut out = Vec::new();
        let rgb = vec![0u8; 2 * 2 * 3];
        let err = WebpEncoder::lossless().encode_rgb8(&rgb, dims(2, 2), &mut out);
        assert!(matches!(err, Err(Error::Unsupported(_))));
    }

    #[test]
    fn lossy_is_unsupported_for_now() {
        let mut out = Vec::new();
        let rgb = vec![0u8; 2 * 3 * 3];
        let err = WebpEncoder::lossy(80).encode_rgb8(&rgb, dims(2, 3), &mut out);
        assert!(matches!(err, Err(Error::Unsupported(_))));
    }

    #[test]
    fn encoder_trait_delegates() {
        let mut out = Vec::new();
        let rgb = vec![0u8; 3];
        let err = (&WebpEncoder::new() as &dyn Encoder).encode(&rgb, dims(1, 1), &mut out);
        assert!(matches!(err, Err(Error::Unsupported(_))));
    }
}
