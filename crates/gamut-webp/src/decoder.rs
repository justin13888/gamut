//! The public WebP decoder: parses the RIFF container and routes to the VP8/VP8L bitstream decoder.
//!
//! Container parsing and format routing are implemented (via [`gamut_riff`]). The lossless **VP8L**
//! and lossy **VP8** bitstreams are decoded natively; an extended **VP8X** file is parsed and its
//! inner bitstream decoded (its `ALPH` alpha chunk is applied in a later milestone).

use gamut_core::{Decoder, Dimensions, Error, Result};
use gamut_riff::{RiffReader, WebpChunkId};

use crate::vp8l::decoder::{argb_to_rgb8, decode as decode_vp8l};

/// Decodes a WebP file to interleaved 8-bit RGB.
///
/// gamut ships its own decoder because every WebP decoder in the Rust ecosystem ultimately wraps
/// libwebp; a `#![forbid(unsafe_code)]` decoder removes that crate's memory-unsafety exposure.
#[derive(Debug, Clone, Default)]
pub struct WebpDecoder {
    /// Reserved for future decode options (e.g. ignoring alpha); keeps the type extensible.
    _private: (),
}

impl WebpDecoder {
    /// Creates a decoder with the default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Decodes the WebP file in `data` to interleaved 8-bit RGB, appending the pixels to `out` and
    /// returning the image [`Dimensions`].
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `data` is not a valid RIFF/WebP container or carries no
    /// recognized bitstream chunk, or [`Error::Unsupported`] for the identified bitstream until its
    /// decode path is implemented.
    pub fn decode_to_rgb8(&self, data: &[u8], out: &mut Vec<u8>) -> Result<Dimensions> {
        // Reconstruction chunks must precede metadata, so the first VP8/VP8L/VP8X chunk wins; any
        // leading metadata/unknown chunks are skipped (RFC 9649 §2.7).
        for chunk in RiffReader::new(data)? {
            let chunk = chunk?;
            match WebpChunkId::from(chunk.fourcc) {
                WebpChunkId::Vp8l => {
                    let (dims, argb) = decode_vp8l(chunk.payload)?;
                    argb_to_rgb8(&argb, out);
                    return Ok(dims);
                }
                WebpChunkId::Vp8 => {
                    let recon = crate::vp8::frame::decode_frame(chunk.payload)?;
                    let yuv = recon.to_yuv420();
                    let dims = Dimensions {
                        width: yuv.width(),
                        height: yuv.height(),
                    };
                    out.extend_from_slice(&yuv.to_rgb8());
                    return Ok(dims);
                }
                WebpChunkId::Vp8x => {
                    // Validate the extended-format header, then fall through to the inner VP8/VP8L
                    // bitstream chunk that follows. Alpha (the `ALPH` chunk gated by the VP8X alpha
                    // flag) is decoded in a later milestone.
                    gamut_riff::Vp8xHeader::from_payload(chunk.payload)?;
                    continue;
                }
                _ => continue,
            }
        }
        Err(Error::InvalidInput(
            "WebP: no VP8/VP8L/VP8X bitstream chunk",
        ))
    }
}

impl Decoder for WebpDecoder {
    fn decode(&self, data: &[u8], out: &mut Vec<u8>) -> Result<Dimensions> {
        self.decode_to_rgb8(data, out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vp8l::bit_io::BitWriter;
    use crate::vp8l::header::Vp8lHeader;
    use crate::vp8l::prefix::write_simple_prefix_code;
    use gamut_riff::{FourCc, RiffWriter, write_simple_lossless, write_simple_lossy};

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
        write_simple_lossless(&w.finish())
    }

    #[test]
    fn decodes_lossless_container_to_rgb8() {
        let file = solid_lossless_webp(2, 2, 0x12, 0x34, 0x56);
        let mut out = Vec::new();
        let dims = WebpDecoder::new().decode_to_rgb8(&file, &mut out).unwrap();
        assert_eq!(
            dims,
            Dimensions {
                width: 2,
                height: 2
            }
        );
        assert_eq!(out, [0x12, 0x34, 0x56].repeat(4));
    }

    #[test]
    fn routes_lossy_container_to_vp8() {
        // A `VP8 ` chunk reaches the VP8 decoder, which rejects this malformed (non-key-frame, 3-byte)
        // payload rather than panicking.
        let file = write_simple_lossy(&[0x9d, 0x01, 0x2a]);
        let mut out = Vec::new();
        assert!(WebpDecoder::new().decode_to_rgb8(&file, &mut out).is_err());
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
        let file = write_extended(&header, &[(FourCc::VP8L, &vp8l)]);
        let mut out = Vec::new();
        let dims = WebpDecoder::new()
            .decode_to_rgb8(&file, &mut out)
            .expect("decode VP8X file");
        assert_eq!(
            dims,
            Dimensions {
                width: 2,
                height: 2
            }
        );
        assert_eq!(out, [0x11, 0x22, 0x33].repeat(4));
    }

    #[test]
    fn rejects_extended_container_without_bitstream() {
        // A VP8X header with no following bitstream chunk has nothing to decode.
        let header = gamut_riff::Vp8xHeader {
            canvas_width: 4,
            canvas_height: 4,
            ..Default::default()
        };
        let file = gamut_riff::write_extended(&header, &[]);
        let mut out = Vec::new();
        assert!(matches!(
            WebpDecoder::new().decode_to_rgb8(&file, &mut out),
            Err(Error::InvalidInput(_))
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
        w.write_chunk(FourCc::ICCP, &[1, 2, 3, 4]);
        w.write_chunk(FourCc::VP8L, &vp8l);
        let file = w.finish();
        let mut out = Vec::new();
        let dims = WebpDecoder::new().decode_to_rgb8(&file, &mut out).unwrap();
        assert_eq!(
            dims,
            Dimensions {
                width: 1,
                height: 1
            }
        );
        assert_eq!(out, [9, 8, 7]);
    }

    #[test]
    fn errors_when_no_bitstream_chunk() {
        let mut w = RiffWriter::new();
        w.write_chunk(FourCc::EXIF, &[0xee; 6]);
        let file = w.finish();
        let mut out = Vec::new();
        let err = WebpDecoder::new().decode_to_rgb8(&file, &mut out);
        assert!(matches!(err, Err(Error::InvalidInput(_))));
    }

    #[test]
    fn rejects_non_riff_data() {
        let mut out = Vec::new();
        let err = (&WebpDecoder::new() as &dyn Decoder).decode(b"not a webp", &mut out);
        assert!(matches!(err, Err(Error::InvalidInput(_))));
    }
}
