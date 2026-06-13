//! The PNG encoder: a [`PngEncoder`] builder implementing [`gamut_core::EncodeImage`] per pixel
//! layout. This phase covers 8-bit truecolour (RGB) with the filter-`None` strategy; further colour
//! types, scanline filters, and space optimisations layer on in later phases.

use gamut_core::{EncodeImage, ImageRef, Result, Rgb8};
use gamut_deflate::{DeflateEncoder, Level};

use crate::chunk::{self, SIGNATURE};
use crate::color::ColorType;
use crate::ihdr;

/// IDAT payload cap. A decoder concatenates consecutive IDATs, so the split is transparent; a
/// large-ish cap keeps the 12-byte per-chunk overhead negligible.
const IDAT_MAX: usize = 1 << 16;

/// A reusable PNG encoder.
#[derive(Debug, Clone)]
pub struct PngEncoder {
    level: Level,
}

impl Default for PngEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl PngEncoder {
    /// Creates an encoder with balanced [`Level::Default`] compression.
    #[must_use]
    pub fn new() -> Self {
        Self {
            level: Level::Default,
        }
    }

    /// Sets the DEFLATE compression [`Level`] used for the image data. [`Level::Best`] is the
    /// space-efficient (slow) setting.
    #[must_use]
    pub fn with_compression(mut self, level: Level) -> Self {
        self.level = level;
        self
    }
}

impl EncodeImage<Rgb8> for PngEncoder {
    fn encode_image(&self, image: ImageRef<'_, Rgb8>, out: &mut Vec<u8>) -> Result<usize> {
        let start = out.len();
        let dims = image.dimensions();
        let row_bytes = dims.width as usize * ColorType::Truecolor.channels();

        out.extend_from_slice(&SIGNATURE);
        ihdr::write(out, dims.width, dims.height, 8, ColorType::Truecolor);

        // Filter every scanline with None (filter-type byte 0, then the raw row).
        let mut filtered = Vec::with_capacity((row_bytes + 1) * dims.height as usize);
        for row in image.as_samples().chunks_exact(row_bytes) {
            filtered.push(0);
            filtered.extend_from_slice(row);
        }

        let mut idat = Vec::new();
        DeflateEncoder::new()
            .with_level(self.level)
            .zlib_compress(&filtered, &mut idat);
        write_idat(out, &idat);

        chunk::write_chunk(out, *b"IEND", &[]);
        Ok(out.len() - start)
    }
}

/// Writes the zlib datastream as one or more consecutive IDAT chunks.
fn write_idat(out: &mut Vec<u8>, zlib_stream: &[u8]) {
    if zlib_stream.is_empty() {
        chunk::write_chunk(out, *b"IDAT", &[]);
        return;
    }
    for piece in zlib_stream.chunks(IDAT_MAX) {
        chunk::write_chunk(out, *b"IDAT", piece);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamut_core::Dimensions;

    #[test]
    fn emits_signature_ihdr_idat_iend() {
        let src = vec![0u8; 2 * 2 * 3];
        let img = ImageRef::<Rgb8>::new(&src, Dimensions::new(2, 2).unwrap()).unwrap();
        let mut png = Vec::new();
        PngEncoder::new().encode_image(img, &mut png).unwrap();
        assert_eq!(&png[..8], &SIGNATURE);
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    }

    #[test]
    fn large_stream_splits_into_multiple_idats() {
        // Incompressible data larger than IDAT_MAX must yield more than one IDAT chunk.
        let mut out = Vec::new();
        let big = vec![0xABu8; IDAT_MAX * 2 + 100];
        write_idat(&mut out, &big);
        let idats = out.windows(4).filter(|w| *w == b"IDAT").count();
        assert!(idats >= 3, "expected multiple IDAT chunks, found {idats}");
    }
}
