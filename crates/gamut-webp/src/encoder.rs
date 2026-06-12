//! The public WebP encoder: orchestrates color handling, the VP8/VP8L bitstream, and the RIFF
//! container, mirroring the shape of [`gamut_avif::AvifEncoder`](https://docs.rs/gamut-avif).
//!
//! Both the lossless **VP8L** path (see [`crate::vp8l::encoder`]) and the lossy **VP8** path are
//! implemented, via the [`EncodeImage<Rgb8>`](gamut_core::EncodeImage) and `EncodeImage<Rgba8>`
//! impls; transparent lossy images use the extended (`VP8X`) format with a raw `ALPH` alpha chunk.

use gamut_color::{Bt601Range, Yuv420};
use gamut_core::{Dimensions, EncodeImage, ImageRef, Result, Rgb8, Rgba8};
use gamut_riff::{FourCc, Vp8xHeader, write_extended, write_simple_lossless, write_simple_lossy};

use crate::alpha;
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
/// [`WebpEncoder::lossy`], then encode via the [`EncodeImage`](gamut_core::EncodeImage) trait.
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

    /// Encodes interleaved 8-bit RGB `pixels` (row-major) of `dims`, appending the WebP file to
    /// `out`. Backs the [`EncodeImage<Rgb8>`] impl; the buffer is already validated by [`ImageRef`].
    fn encode_rgb8_inner(
        &self,
        pixels: &[u8],
        dims: Dimensions,
        out: &mut Vec<u8>,
    ) -> Result<usize> {
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
                // WebP/VP8 is limited-range BT.601 (what libwebp + browsers decode); see Bt601Range.
                let yuv = Yuv420::from_rgb8(pixels, dims.width, dims.height, Bt601Range::Limited)?;
                let (payload, _recon) = encode_frame(&yuv, quality_to_quant(self.config.quality));
                let file = write_simple_lossy(&payload);
                let written = file.len();
                out.extend_from_slice(&file);
                Ok(written)
            }
        }
    }

    /// Encodes interleaved 8-bit RGBA `pixels` (row-major) of `dims`, appending the WebP file to
    /// `out`. A fully opaque image produces a simple file; a transparent one uses the extended
    /// (`VP8X`) format with a raw `ALPH` alpha chunk (lossy color) or in-bitstream alpha (lossless).
    /// Backs the [`EncodeImage<Rgba8>`] impl; the buffer is already validated by [`ImageRef`].
    fn encode_rgba8_inner(
        &self,
        pixels: &[u8],
        dims: Dimensions,
        out: &mut Vec<u8>,
    ) -> Result<usize> {
        let file = match self.config.mode {
            WebpMode::Lossless => {
                let argb: Vec<u32> = pixels
                    .chunks_exact(4)
                    .map(|p| make_argb(p[3], p[0], p[1], p[2]))
                    .collect();
                write_simple_lossless(&encode_vp8l(&argb, dims)?)
            }
            WebpMode::Lossy => {
                let rgb: Vec<u8> = pixels
                    .chunks_exact(4)
                    .flat_map(|p| [p[0], p[1], p[2]])
                    .collect();
                let yuv = Yuv420::from_rgb8(&rgb, dims.width, dims.height, Bt601Range::Limited)?;
                let (vp8, _) = encode_frame(&yuv, quality_to_quant(self.config.quality));
                if pixels.chunks_exact(4).all(|p| p[3] == 0xff) {
                    write_simple_lossy(&vp8)
                } else {
                    let alpha: Vec<u8> = pixels.chunks_exact(4).map(|p| p[3]).collect();
                    let alph =
                        alpha::write_alph(&alpha, dims.width as usize, dims.height as usize)?;
                    let header = Vp8xHeader {
                        alpha: true,
                        canvas_width: dims.width,
                        canvas_height: dims.height,
                        ..Default::default()
                    };
                    write_extended(&header, &[(FourCc::ALPH, &alph), (FourCc::VP8, &vp8)])
                }
            }
        };
        let written = file.len();
        out.extend_from_slice(&file);
        Ok(written)
    }
}

impl EncodeImage<Rgb8> for WebpEncoder {
    fn encode_image(&self, image: ImageRef<'_, Rgb8>, out: &mut Vec<u8>) -> Result<usize> {
        self.encode_rgb8_inner(image.as_samples(), image.dimensions(), out)
    }
}

impl EncodeImage<Rgba8> for WebpEncoder {
    fn encode_image(&self, image: ImageRef<'_, Rgba8>, out: &mut Vec<u8>) -> Result<usize> {
        self.encode_rgba8_inner(image.as_samples(), image.dimensions(), out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamut_core::{DecodeImage, ImageBuf};

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
        // Validation now lives at the ImageRef boundary, before the encoder is even called.
        assert!(ImageRef::<Rgb8>::new(&[0u8; 10], dims(2, 2)).is_err());
    }

    #[test]
    fn lossless_encodes_a_valid_webp_file() {
        // A solid 2x2 RGB image encodes to a RIFF/WebP file that the gamut decoder reads back
        // bit-exactly (the round-trip is the lossless guarantee).
        let mut out = Vec::new();
        let rgb = [0x10, 0x20, 0x30].repeat(4);
        let written = WebpEncoder::lossless()
            .encode_image(ImageRef::<Rgb8>::new(&rgb, dims(2, 2)).unwrap(), &mut out)
            .expect("encode");
        assert_eq!(written, out.len());
        assert_eq!(&out[0..4], b"RIFF");

        let decoded: ImageBuf<Rgb8> = crate::WebpDecoder::new()
            .decode_image(&out)
            .expect("decode");
        assert_eq!(decoded.dimensions(), dims(2, 2));
        assert_eq!(decoded.as_samples(), rgb.as_slice());
    }

    #[test]
    fn lossy_encodes_a_decodable_webp_file() {
        // Lossy now produces a RIFF/WebP the native decoder reads back to RGB of the right shape (the
        // pixels are lossy, so only structure is checked here; bit-exactness is the libwebp oracle).
        let mut out = Vec::new();
        let rgb = [40u8, 80, 120].repeat(16 * 16);
        let written = WebpEncoder::lossy(60)
            .encode_image(ImageRef::<Rgb8>::new(&rgb, dims(16, 16)).unwrap(), &mut out)
            .expect("lossy encode");
        assert_eq!(written, out.len());
        assert_eq!(&out[0..4], b"RIFF");
        let decoded: ImageBuf<Rgb8> = crate::WebpDecoder::new()
            .decode_image(&out)
            .expect("decode");
        assert_eq!(decoded.dimensions(), dims(16, 16));
        assert_eq!(decoded.as_samples().len(), 16 * 16 * 3);
    }

    #[test]
    fn lossy_rgba_round_trips_alpha_exactly() {
        // Transparent content: the alpha is stored losslessly (raw `ALPH`), so it round-trips
        // bit-exactly through the extended container; only the color is lossy.
        let (w, h) = (32u32, 24u32);
        let rgba: Vec<u8> = (0..(w * h) as usize)
            .flat_map(|i| {
                let (x, y) = (i as u32 % w, i as u32 / w);
                [
                    (x * 7) as u8,
                    (y * 9) as u8,
                    (x ^ y) as u8,
                    ((x * 5 + y * 3) & 0xff) as u8,
                ]
            })
            .collect();
        let mut file = Vec::new();
        WebpEncoder::lossy(75)
            .encode_image(
                ImageRef::<Rgba8>::new(&rgba, dims(w, h)).unwrap(),
                &mut file,
            )
            .expect("rgba encode");
        assert_eq!(&file[0..4], b"RIFF");

        let decoded: ImageBuf<Rgba8> = crate::WebpDecoder::new()
            .decode_image(&file)
            .expect("rgba decode");
        assert_eq!(decoded.dimensions(), dims(w, h));
        let dec_alpha: Vec<u8> = decoded.as_samples().chunks_exact(4).map(|p| p[3]).collect();
        let src_alpha: Vec<u8> = rgba.chunks_exact(4).map(|p| p[3]).collect();
        assert_eq!(dec_alpha, src_alpha, "alpha must round-trip losslessly");
    }

    #[test]
    fn opaque_rgba_uses_the_simple_lossy_format() {
        use gamut_riff::{RiffReader, WebpChunkId};
        let rgba = [120u8, 60, 200, 0xff].repeat(16 * 16);
        let mut file = Vec::new();
        WebpEncoder::lossy(60)
            .encode_image(
                ImageRef::<Rgba8>::new(&rgba, dims(16, 16)).unwrap(),
                &mut file,
            )
            .expect("rgba encode");
        // A fully-opaque image carries no alpha overhead — just a single `VP8 ` chunk.
        let ids: Vec<_> = RiffReader::new(&file)
            .unwrap()
            .map(|c| WebpChunkId::from(c.unwrap().fourcc))
            .collect();
        assert_eq!(ids, vec![WebpChunkId::Vp8]);
    }

    #[test]
    fn encode_image_is_object_safe() {
        let mut out = Vec::new();
        let rgb = [7u8, 8, 9];
        let enc: &dyn EncodeImage<Rgb8> = &WebpEncoder::new();
        let written = enc
            .encode_image(ImageRef::<Rgb8>::new(&rgb, dims(1, 1)).unwrap(), &mut out)
            .expect("encode via trait");
        assert_eq!(written, out.len());
        assert_eq!(&out[0..4], b"RIFF");
    }
}
