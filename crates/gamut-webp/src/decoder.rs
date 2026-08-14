//! The public WebP decoder: parses the RIFF container and routes to the VP8/VP8L bitstream decoder.
//!
//! Container parsing and format routing are implemented (via [`gamut_riff`]). The lossless **VP8L**
//! and lossy **VP8** bitstreams are decoded natively; an extended **VP8X** file is parsed and its
//! inner bitstream decoded. Decoding to [`Rgba8`](gamut_core::Rgba8) applies a lossy file's `ALPH`
//! alpha chunk and preserves a VP8L stream's own alpha; decoding to [`Rgb8`](gamut_core::Rgb8)
//! drops alpha.

use std::fmt;
use std::sync::{Arc, Mutex};

use gamut_color::{ColorRange, Yuv420};
use gamut_core::{DecodeImage, Dimensions, Error, ImageBuf, Result, Rgb8, Rgba8};
use gamut_riff::{WebpChunkId, WebpLayout};

use crate::alpha;
use crate::backend::{
    CodestreamInfo, DecodedRaster, SharedDecoder, WebpCodestream, WebpCodestreamDecoder,
    dispatch_decode, peek_dimensions,
};
use crate::vp8l::decoder::{argb_to_rgb8, argb_to_rgba8, decode as decode_vp8l};

/// Decodes a WebP file to interleaved 8-bit RGB.
///
/// gamut ships its own decoder because every WebP decoder in the Rust ecosystem ultimately wraps
/// libwebp; a `#![forbid(unsafe_code)]` decoder removes that crate's memory-unsafety exposure.
/// The codestream itself may be decoded by a pluggable backend installed with
/// [`push_backend`](Self::push_backend) — a hardware VP8 decoder, say; with none installed (the
/// default) the crate's own `vp8`/`vp8l` decoders run. See [`crate::backend`] for the fallback
/// contract.
#[derive(Clone, Default)]
pub struct WebpDecoder {
    /// Pluggable codestream decoders, tried in push order ahead of the built-in tails.
    backends: Vec<SharedDecoder>,
}

impl fmt::Debug for WebpDecoder {
    /// Renders the number of installed backends (a backend need not be `Debug`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebpDecoder")
            .field("backends", &self.backends.len())
            .finish()
    }
}

impl WebpDecoder {
    /// Creates a decoder with the default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Installs a codestream decoder backend, returning `&mut self` so pushes chain.
    ///
    /// Backends are tried in **push order**, ahead of the built-in `vp8`/`vp8l` decoders, which
    /// remain the implicit tails and cannot be removed. A backend declines a job by returning
    /// `false` from [`supports`](WebpCodestreamDecoder::supports); once it accepts, its error
    /// propagates and no other decoder is tried.
    ///
    /// **Cloning a `WebpDecoder` shares its backends**: the registry holds each backend behind an
    /// [`Arc`], so a clone dispatches to the very same backend objects (and the same interior
    /// state), it does not copy them.
    pub fn push_backend(&mut self, backend: impl WebpCodestreamDecoder + 'static) -> &mut Self {
        self.backends.push(Arc::new(Mutex::new(backend)));
        self
    }

    /// Decodes one codestream `payload` through the registry, falling back to the built-in decoder
    /// when every backend declines (or when the codestream header cannot even be peeked, in which
    /// case the built-in decoder reports the parse error).
    fn decode_codestream(
        &self,
        codestream: WebpCodestream,
        payload: &[u8],
    ) -> Result<DecodedRaster> {
        if !self.backends.is_empty()
            && let Some(dims) = peek_dimensions(codestream, payload)
            && let Some(result) = dispatch_decode(
                &self.backends,
                &CodestreamInfo::new(codestream, dims),
                payload,
            )
        {
            return result;
        }
        match codestream {
            WebpCodestream::Vp8 => Ok(DecodedRaster::Yuv420(
                crate::vp8::frame::decode_frame(payload)?.to_yuv420(),
            )),
            WebpCodestream::Vp8l => {
                let (dimensions, pixels) = decode_vp8l(payload)?;
                Ok(DecodedRaster::Argb { dimensions, pixels })
            }
        }
    }

    /// Unwraps a lossless decode result, which is ARGB by construction (the registry rejects a
    /// backend that returns the other variant).
    fn expect_argb(raster: DecodedRaster) -> Result<(Dimensions, Vec<u32>)> {
        match raster {
            DecodedRaster::Argb { dimensions, pixels } => Ok((dimensions, pixels)),
            DecodedRaster::Yuv420(_) => Err(Error::invalid_input(
                env!("CARGO_PKG_NAME"),
                "WebP: VP8L decode produced a YUV raster",
            )),
        }
    }

    /// Unwraps a lossy decode result, which is YUV 4:2:0 by construction.
    fn expect_yuv(raster: DecodedRaster) -> Result<Yuv420> {
        match raster {
            DecodedRaster::Yuv420(yuv) => Ok(yuv),
            DecodedRaster::Argb { .. } => Err(Error::invalid_input(
                env!("CARGO_PKG_NAME"),
                "WebP: VP8 decode produced an ARGB raster",
            )),
        }
    }

    /// Sorts the container's chunks and locates the image bitstream.
    ///
    /// [`WebpLayout::parse`] is the single container walk behind both pixel paths: it validates the
    /// `VP8X` header, enforces the spec's reconstruction-chunk order, and hands back the `ALPH` and
    /// bitstream payloads (RFC 9649 §2.7).
    fn layout(data: &[u8]) -> Result<(WebpLayout<'_>, WebpCodestream, &[u8])> {
        let layout = WebpLayout::parse(data)?;
        let (id, payload) = layout.bitstream.ok_or_else(|| {
            Error::invalid_input(env!("CARGO_PKG_NAME"), "WebP: no VP8/VP8L bitstream chunk")
        })?;
        let codestream = match id {
            WebpChunkId::Vp8l => WebpCodestream::Vp8l,
            _ => WebpCodestream::Vp8,
        };
        Ok((layout, codestream, payload))
    }

    /// Decodes the WebP file in `data` to interleaved 8-bit RGB, appending the pixels to `out` and
    /// returning the image [`Dimensions`]. Backs the [`DecodeImage<Rgb8>`] impl.
    ///
    /// This path carries no alpha, so an `ALPH` chunk is ignored here; the RGBA decoder applies it
    /// (see `decode_rgba8_into`).
    fn decode_rgb8_into(&self, data: &[u8], out: &mut Vec<u8>) -> Result<Dimensions> {
        let (_, codestream, payload) = Self::layout(data)?;
        match codestream {
            WebpCodestream::Vp8l => {
                let (dims, argb) =
                    Self::expect_argb(self.decode_codestream(WebpCodestream::Vp8l, payload)?)?;
                argb_to_rgb8(&argb, out);
                Ok(dims)
            }
            WebpCodestream::Vp8 => {
                let yuv = Self::expect_yuv(self.decode_codestream(WebpCodestream::Vp8, payload)?)?;
                let dims = Dimensions {
                    width: yuv.width(),
                    height: yuv.height(),
                };
                // WebP/VP8 is limited-range BT.601; decode with the matching inverse.
                out.extend_from_slice(&yuv.to_rgb8(ColorRange::Limited));
                Ok(dims)
            }
        }
    }

    /// Decodes the WebP file in `data` to interleaved 8-bit RGBA, appending the pixels to `out` and
    /// returning the image [`Dimensions`]. A simple (alpha-less) file decodes to opaque RGBA; an
    /// extended file's `ALPH` chunk supplies the alpha; a `VP8L` bitstream carries its own. Backs the
    /// [`DecodeImage<Rgba8>`] impl.
    fn decode_rgba8_into(&self, data: &[u8], out: &mut Vec<u8>) -> Result<Dimensions> {
        let (layout, codestream, payload) = Self::layout(data)?;
        match codestream {
            WebpCodestream::Vp8l => {
                let (dims, argb) =
                    Self::expect_argb(self.decode_codestream(WebpCodestream::Vp8l, payload)?)?;
                argb_to_rgba8(&argb, out);
                Ok(dims)
            }
            WebpCodestream::Vp8 => {
                let yuv = Self::expect_yuv(self.decode_codestream(WebpCodestream::Vp8, payload)?)?;
                let dims = Dimensions {
                    width: yuv.width(),
                    height: yuv.height(),
                };
                let (w, h) = (dims.width as usize, dims.height as usize);
                let alpha = match layout.alph {
                    Some(payload) => alpha::read_alph(payload, w, h)?,
                    None => vec![0xffu8; w * h],
                };
                let rgb = yuv.to_rgb8(ColorRange::Limited);
                for (px, &a) in rgb.chunks_exact(3).zip(alpha.iter()) {
                    out.extend_from_slice(&[px[0], px[1], px[2], a]);
                }
                Ok(dims)
            }
        }
    }
}

impl DecodeImage<Rgb8> for WebpDecoder {
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<Rgb8>> {
        let mut px = Vec::new();
        let dims = self.decode_rgb8_into(data, &mut px)?;
        ImageBuf::new(px, dims)
    }
}

impl DecodeImage<Rgba8> for WebpDecoder {
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<Rgba8>> {
        let mut px = Vec::new();
        let dims = self.decode_rgba8_into(data, &mut px)?;
        ImageBuf::new(px, dims)
    }
}

#[cfg(test)]
mod tests {
    use gamut_riff::{FourCc, RiffReader, RiffWriter, write_simple_lossless, write_simple_lossy};

    use super::*;
    use crate::vp8l::bit_io::BitWriter;
    use crate::vp8l::header::Vp8lHeader;
    use crate::vp8l::prefix::write_simple_prefix_code;

    /// Builds a simple-lossless WebP file holding a solid-color `width`×`height` VP8L image.
    fn solid_lossless_webp(width: u32, height: u32, r: u8, g: u8, b: u8) -> Vec<u8> {
        let mut w = BitWriter::new();
        Vp8lHeader::from_dimensions(Dimensions { width, height }, false)
            .unwrap()
            .write(&mut w);
        w.write_bits(0, 1); // no transforms
        w.write_bits(0, 1); // no color cache
        w.write_bits(0, 1); // single meta prefix code
        write_simple_prefix_code(&mut w, &[u16::from(g)]);
        write_simple_prefix_code(&mut w, &[u16::from(r)]);
        write_simple_prefix_code(&mut w, &[u16::from(b)]);
        write_simple_prefix_code(&mut w, &[0xff]); // alpha (opaque)
        write_simple_prefix_code(&mut w, &[0]); // distance (unused)
        write_simple_lossless(&w.finish()).unwrap()
    }

    #[test]
    fn decodes_lossless_container_to_rgb8() {
        let file = solid_lossless_webp(2, 2, 0x12, 0x34, 0x56);
        let got: ImageBuf<Rgb8> = WebpDecoder::new().decode_image(&file).unwrap();
        assert_eq!(
            got.dimensions(),
            Dimensions {
                width: 2,
                height: 2
            }
        );
        assert_eq!(got.as_samples(), [0x12, 0x34, 0x56].repeat(4).as_slice());
    }

    #[test]
    fn routes_lossy_container_to_vp8() {
        // A `VP8 ` chunk reaches the VP8 decoder, which rejects this malformed (non-key-frame, 3-byte)
        // payload rather than panicking.
        let file = write_simple_lossy(&[0x9d, 0x01, 0x2a]).unwrap();
        let got: Result<ImageBuf<Rgb8>> = WebpDecoder::new().decode_image(&file);
        assert!(got.is_err());
    }

    #[test]
    fn decodes_extended_container_with_inner_bitstream() {
        use gamut_riff::{Vp8xHeader, write_extended};
        // A VP8X feature header followed by a VP8L bitstream decodes to the inner image (the alpha
        // flag's `ALPH` chunk is handled in a later milestone).
        let inner = solid_lossless_webp(2, 2, 0x11, 0x22, 0x33);
        let vp8l = RiffReader::new(&inner)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .payload
            .to_vec();
        let header = Vp8xHeader {
            canvas_width: 2,
            canvas_height: 2,
            ..Default::default()
        };
        let file = write_extended(&header, &[(FourCc::VP8L, &vp8l)]).unwrap();
        let got: ImageBuf<Rgb8> = WebpDecoder::new()
            .decode_image(&file)
            .expect("decode VP8X file");
        assert_eq!(
            got.dimensions(),
            Dimensions {
                width: 2,
                height: 2
            }
        );
        assert_eq!(got.as_samples(), [0x11, 0x22, 0x33].repeat(4).as_slice());
    }

    #[test]
    fn rejects_extended_container_without_bitstream() {
        // A VP8X header with no following bitstream chunk has nothing to decode.
        let header = gamut_riff::Vp8xHeader {
            canvas_width: 4,
            canvas_height: 4,
            ..Default::default()
        };
        let file = gamut_riff::write_extended(&header, &[]).unwrap();
        let got: Result<ImageBuf<Rgb8>> = WebpDecoder::new().decode_image(&file);
        assert!(matches!(
            got,
            Err(error) if error.kind() == gamut_core::ErrorKind::InvalidInput
        ));
    }

    #[test]
    fn skips_leading_metadata_then_decodes_bitstream() {
        // A leading metadata chunk must be skipped; the VP8L chunk that follows is decoded.
        let vp8l = {
            let full = solid_lossless_webp(1, 1, 9, 8, 7);
            // Extract just the VP8L chunk payload from the simple-lossless file.
            RiffReader::new(&full)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .payload
                .to_vec()
        };
        let mut w = RiffWriter::new();
        w.write_chunk(FourCc::ICCP, &[1, 2, 3, 4]).unwrap();
        w.write_chunk(FourCc::VP8L, &vp8l).unwrap();
        let file = w.finish().unwrap();
        let got: ImageBuf<Rgb8> = WebpDecoder::new().decode_image(&file).unwrap();
        assert_eq!(
            got.dimensions(),
            Dimensions {
                width: 1,
                height: 1
            }
        );
        assert_eq!(got.as_samples(), [9, 8, 7].as_slice());
    }

    #[test]
    fn errors_when_no_bitstream_chunk() {
        let mut w = RiffWriter::new();
        w.write_chunk(FourCc::EXIF, &[0xee; 6]).unwrap();
        let file = w.finish().unwrap();
        let err: Result<ImageBuf<Rgb8>> = WebpDecoder::new().decode_image(&file);
        assert!(matches!(
            err,
            Err(error) if error.kind() == gamut_core::ErrorKind::InvalidInput
        ));
    }

    #[test]
    fn rejects_non_riff_data() {
        let err: Result<ImageBuf<Rgb8>> = WebpDecoder::new().decode_image(b"not a webp");
        assert!(matches!(
            err,
            Err(error) if error.kind() == gamut_core::ErrorKind::InvalidInput
        ));
    }

    #[test]
    fn decodes_lossless_container_to_rgba8() {
        // A VP8L file decoded to RGBA carries the stream's own alpha (opaque here). Pins the VP8L arm
        // of the RGBA decoder, which deleting would route to "no bitstream".
        let file = solid_lossless_webp(2, 2, 0x12, 0x34, 0x56);
        let got: ImageBuf<Rgba8> = WebpDecoder::new().decode_image(&file).unwrap();
        assert_eq!(
            got.dimensions(),
            Dimensions {
                width: 2,
                height: 2
            }
        );
        assert_eq!(
            got.as_samples(),
            [0x12, 0x34, 0x56, 0xff].repeat(4).as_slice()
        );
    }

    #[test]
    fn rejects_malformed_vp8x_header() {
        // A VP8X chunk with a too-short payload (a valid one is 10 bytes) is malformed: the decoder
        // must parse-and-reject it, not silently skip to the inner bitstream. Pins the VP8X arm of
        // both the RGB and RGBA paths.
        let inner = solid_lossless_webp(2, 2, 1, 2, 3);
        let vp8l = RiffReader::new(&inner)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .payload
            .to_vec();
        let mut w = RiffWriter::new();
        w.write_chunk(FourCc::VP8X, &[0u8; 4]).unwrap();
        w.write_chunk(FourCc::VP8L, &vp8l).unwrap();
        let file = w.finish().unwrap();
        let rgb: Result<ImageBuf<Rgb8>> = WebpDecoder::new().decode_image(&file);
        assert!(
            rgb.is_err(),
            "RGB decode must reject a malformed VP8X header"
        );
        let rgba: Result<ImageBuf<Rgba8>> = WebpDecoder::new().decode_image(&file);
        assert!(
            rgba.is_err(),
            "RGBA decode must reject a malformed VP8X header"
        );
    }
}
