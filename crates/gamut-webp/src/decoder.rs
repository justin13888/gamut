//! The public WebP decoder: parses the RIFF container and routes to the VP8/VP8L bitstream decoder.
//!
//! Container parsing and format routing are implemented (via [`gamut_riff`]); the bitstream decoders
//! are under construction (tracked in `../STATUS.md`), so [`WebpDecoder`] identifies the bitstream
//! and then returns [`Error::Unsupported`] until the VP8L M0 path lands.

use gamut_core::{Decoder, Dimensions, Error, Result};
use gamut_riff::{RiffReader, WebpChunkId};

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
            match WebpChunkId::from(chunk?.fourcc) {
                WebpChunkId::Vp8l => {
                    return Err(Error::Unsupported(
                        "WebP VP8L (lossless) decoding not yet implemented",
                    ));
                }
                WebpChunkId::Vp8 => {
                    return Err(Error::Unsupported(
                        "WebP VP8 (lossy) decoding not yet implemented",
                    ));
                }
                WebpChunkId::Vp8x => {
                    return Err(Error::Unsupported(
                        "extended WebP (VP8X) decoding not yet implemented",
                    ));
                }
                _ => continue,
            }
        }
        let _ = out;
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
    use gamut_riff::{FourCc, RiffWriter, write_simple_lossless, write_simple_lossy};

    #[test]
    fn routes_lossless_container_to_vp8l() {
        let file = write_simple_lossless(&[0x2f, 0, 0, 0]);
        let mut out = Vec::new();
        let err = WebpDecoder::new().decode_to_rgb8(&file, &mut out);
        assert!(matches!(err, Err(Error::Unsupported(m)) if m.contains("VP8L")));
    }

    #[test]
    fn routes_lossy_container_to_vp8() {
        let file = write_simple_lossy(&[0x9d, 0x01, 0x2a]);
        let mut out = Vec::new();
        let err = WebpDecoder::new().decode_to_rgb8(&file, &mut out);
        assert!(matches!(err, Err(Error::Unsupported(m)) if m.contains("VP8 (lossy)")));
    }

    #[test]
    fn routes_extended_container_to_vp8x() {
        let mut w = RiffWriter::new();
        w.write_chunk(FourCc::VP8X, &[0u8; 10]);
        let file = w.finish();
        let mut out = Vec::new();
        let err = WebpDecoder::new().decode_to_rgb8(&file, &mut out);
        assert!(matches!(err, Err(Error::Unsupported(m)) if m.contains("VP8X")));
    }

    #[test]
    fn skips_leading_metadata_then_finds_bitstream() {
        let mut w = RiffWriter::new();
        w.write_chunk(FourCc::ICCP, &[1, 2, 3, 4]);
        w.write_chunk(FourCc::VP8L, &[0x2f, 0, 0, 0]);
        let file = w.finish();
        let mut out = Vec::new();
        let err = WebpDecoder::new().decode_to_rgb8(&file, &mut out);
        assert!(matches!(err, Err(Error::Unsupported(m)) if m.contains("VP8L")));
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
