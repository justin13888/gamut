//! The public WebP encoder: orchestrates color handling, the VP8/VP8L bitstream, and the RIFF
//! container, mirroring the shape of [`gamut_avif::AvifEncoder`](https://docs.rs/gamut-avif).
//!
//! The lossless **VP8L** path is implemented (see [`crate::vp8l::encoder`]); the lossy **VP8** path
//! still returns [`Error::Unsupported`].

use gamut_color::Yuv420;
use gamut_core::{Dimensions, Encoder, Error, Result};
use gamut_riff::{write_simple_lossless, write_simple_lossy};

use crate::config::{WebpConfig, WebpMode};
use crate::vp8::frame::encode_frame;
use crate::vp8l::encoder::encode as encode_vp8l;
use crate::vp8l::transform::make_argb;

/// Maps a `0..=100` quality to a VP8 base quantizer index (`0..=127`); higher quality → lower index
/// (less quantization). This is the keystone's simple mapping; finer rate control is issue #32.
fn quality_to_quant(quality: u8) -> u8 {
    let q = u32::from(quality.min(100));
    ((100 - q) * 127 / 100) as u8
}

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
        match self.config.mode {
            WebpMode::Lossless => {
                let argb: Vec<u32> = pixels
                    .chunks_exact(3)
                    .map(|p| make_argb(0xff, p[0], p[1], p[2]))
                    .collect();
                let bitstream = encode_vp8l(&argb, dims)?;
                let file = write_simple_lossless(&bitstream);
                let written = file.len();
                out.extend_from_slice(&file);
                Ok(written)
            }
            WebpMode::Lossy => {
                let yuv = Yuv420::from_rgb8(pixels, dims.width, dims.height)?;
                let (payload, _recon) = encode_frame(&yuv, quality_to_quant(self.config.quality));
                let file = write_simple_lossy(&payload);
                let written = file.len();
                out.extend_from_slice(&file);
                Ok(written)
            }
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
    fn lossless_encodes_a_valid_webp_file() {
        // A solid 2x2 RGB image encodes to a RIFF/WebP file that the gamut decoder reads back
        // bit-exactly (the round-trip is the lossless guarantee).
        let mut out = Vec::new();
        let rgb = [0x10, 0x20, 0x30].repeat(4);
        let written = WebpEncoder::lossless()
            .encode_rgb8(&rgb, dims(2, 2), &mut out)
            .expect("encode");
        assert_eq!(written, out.len());
        assert_eq!(&out[0..4], b"RIFF");

        let mut decoded = Vec::new();
        let d = crate::WebpDecoder::new()
            .decode_to_rgb8(&out, &mut decoded)
            .expect("decode");
        assert_eq!(d, dims(2, 2));
        assert_eq!(decoded, rgb);
    }

    #[test]
    fn lossy_encodes_a_decodable_webp_file() {
        // Lossy now produces a RIFF/WebP the native decoder reads back to RGB of the right shape (the
        // pixels are lossy, so only structure is checked here; bit-exactness is the libwebp oracle).
        let mut out = Vec::new();
        let rgb = [40u8, 80, 120].repeat(16 * 16);
        let written = WebpEncoder::lossy(60)
            .encode_rgb8(&rgb, dims(16, 16), &mut out)
            .expect("lossy encode");
        assert_eq!(written, out.len());
        assert_eq!(&out[0..4], b"RIFF");
        let mut decoded = Vec::new();
        let d = crate::WebpDecoder::new()
            .decode_to_rgb8(&out, &mut decoded)
            .expect("decode");
        assert_eq!(d, dims(16, 16));
        assert_eq!(decoded.len(), 16 * 16 * 3);
    }

    #[test]
    fn encoder_trait_delegates() {
        let mut out = Vec::new();
        let rgb = [7u8, 8, 9];
        let written = (&WebpEncoder::new() as &dyn Encoder)
            .encode(&rgb, dims(1, 1), &mut out)
            .expect("encode via trait");
        assert_eq!(written, out.len());
        assert_eq!(&out[0..4], b"RIFF");
    }
}
