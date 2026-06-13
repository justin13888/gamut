//! The PNG encoder: a [`PngEncoder`] builder implementing [`gamut_core::EncodeImage`] for each
//! supported pixel layout. This covers the four non-indexed colour types at 8- and 16-bit depth;
//! palette, sub-byte depths, ancillary chunks, and space optimisations layer on in later phases.

use gamut_core::{
    EncodeImage, Gray8, Gray16, GrayAlpha8, GrayAlpha16, ImageRef, Pixel, Result, Rgb8, Rgb16,
    Rgba8, Rgba16,
};
use gamut_deflate::{DeflateEncoder, Level};

use crate::chunk::{self, SIGNATURE};
use crate::color::ColorType;
use crate::filter::{self, FilterStrategy};
use crate::ihdr;

/// IDAT payload cap. A decoder concatenates consecutive IDATs, so the split is transparent; a
/// large-ish cap keeps the 12-byte per-chunk overhead negligible.
const IDAT_MAX: usize = 1 << 16;

/// A reusable PNG encoder.
#[derive(Debug, Clone)]
pub struct PngEncoder {
    level: Level,
    filter: FilterStrategy,
}

impl Default for PngEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl PngEncoder {
    /// Creates an encoder with balanced [`Level::Default`] compression and the
    /// [`FilterStrategy::MinSumAbs`] filter heuristic.
    #[must_use]
    pub fn new() -> Self {
        Self {
            level: Level::Default,
            filter: FilterStrategy::MinSumAbs,
        }
    }

    /// Sets the DEFLATE compression [`Level`] used for the image data. [`Level::Best`] is the
    /// space-efficient (slow) setting.
    #[must_use]
    pub fn with_compression(mut self, level: Level) -> Self {
        self.level = level;
        self
    }

    /// Sets the scanline [`FilterStrategy`].
    #[must_use]
    pub fn with_filter(mut self, filter: FilterStrategy) -> Self {
        self.filter = filter;
        self
    }

    /// Encodes an 8-bit-per-sample image (samples are already PNG's storage bytes).
    fn encode_8bit<P: Pixel<Sample = u8>>(
        &self,
        image: ImageRef<'_, P>,
        color: ColorType,
        out: &mut Vec<u8>,
    ) -> usize {
        let dims = image.dimensions();
        self.write_png((dims.width, dims.height), image.as_samples(), color, 8, out)
    }

    /// Encodes a 16-bit-per-sample image, serialising samples big-endian (PNG's network byte order).
    fn encode_16bit<P: Pixel<Sample = u16>>(
        &self,
        image: ImageRef<'_, P>,
        color: ColorType,
        out: &mut Vec<u8>,
    ) -> usize {
        let dims = image.dimensions();
        let samples = image.as_samples();
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for &sample in samples {
            bytes.extend_from_slice(&sample.to_be_bytes());
        }
        self.write_png((dims.width, dims.height), &bytes, color, 16, out)
    }

    /// Shared back end: signature → IHDR → filtered + DEFLATE-compressed scanlines as IDAT(s) → IEND.
    /// `sample_bytes` is the image in PNG storage order; the stride is derived from `color` and
    /// `bit_depth`.
    fn write_png(
        &self,
        (width, height): (u32, u32),
        sample_bytes: &[u8],
        color: ColorType,
        bit_depth: u8,
        out: &mut Vec<u8>,
    ) -> usize {
        let bpp = color.channels() * (bit_depth as usize / 8);
        let row_bytes = width as usize * bpp;

        let start = out.len();
        out.extend_from_slice(&SIGNATURE);
        ihdr::write(out, width, height, bit_depth, color);

        let filtered = filter::filter_image(self.filter, sample_bytes, row_bytes, bpp);
        let mut idat = Vec::new();
        DeflateEncoder::new()
            .with_level(self.level)
            .zlib_compress(&filtered, &mut idat);
        write_idat(out, &idat);

        chunk::write_chunk(out, *b"IEND", &[]);
        out.len() - start
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

// One impl per supported pixel layout. Indexed colour is handled separately (it needs a palette);
// CMYK has no PNG colour type.
impl EncodeImage<Gray8> for PngEncoder {
    fn encode_image(&self, image: ImageRef<'_, Gray8>, out: &mut Vec<u8>) -> Result<usize> {
        Ok(self.encode_8bit(image, ColorType::Grayscale, out))
    }
}
impl EncodeImage<Rgb8> for PngEncoder {
    fn encode_image(&self, image: ImageRef<'_, Rgb8>, out: &mut Vec<u8>) -> Result<usize> {
        Ok(self.encode_8bit(image, ColorType::Truecolor, out))
    }
}
impl EncodeImage<Rgba8> for PngEncoder {
    fn encode_image(&self, image: ImageRef<'_, Rgba8>, out: &mut Vec<u8>) -> Result<usize> {
        Ok(self.encode_8bit(image, ColorType::TruecolorAlpha, out))
    }
}
impl EncodeImage<GrayAlpha8> for PngEncoder {
    fn encode_image(&self, image: ImageRef<'_, GrayAlpha8>, out: &mut Vec<u8>) -> Result<usize> {
        Ok(self.encode_8bit(image, ColorType::GrayscaleAlpha, out))
    }
}
impl EncodeImage<Gray16> for PngEncoder {
    fn encode_image(&self, image: ImageRef<'_, Gray16>, out: &mut Vec<u8>) -> Result<usize> {
        Ok(self.encode_16bit(image, ColorType::Grayscale, out))
    }
}
impl EncodeImage<Rgb16> for PngEncoder {
    fn encode_image(&self, image: ImageRef<'_, Rgb16>, out: &mut Vec<u8>) -> Result<usize> {
        Ok(self.encode_16bit(image, ColorType::Truecolor, out))
    }
}
impl EncodeImage<Rgba16> for PngEncoder {
    fn encode_image(&self, image: ImageRef<'_, Rgba16>, out: &mut Vec<u8>) -> Result<usize> {
        Ok(self.encode_16bit(image, ColorType::TruecolorAlpha, out))
    }
}
impl EncodeImage<GrayAlpha16> for PngEncoder {
    fn encode_image(&self, image: ImageRef<'_, GrayAlpha16>, out: &mut Vec<u8>) -> Result<usize> {
        Ok(self.encode_16bit(image, ColorType::GrayscaleAlpha, out))
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
    fn ihdr_reports_color_type_and_depth() {
        // A 16-bit grayscale-alpha image should declare colour type 4, bit depth 16.
        let src = vec![0u16; 3 * 3 * 2];
        let img = ImageRef::<GrayAlpha16>::new(&src, Dimensions::new(3, 3).unwrap()).unwrap();
        let mut png = Vec::new();
        PngEncoder::new().encode_image(img, &mut png).unwrap();
        // IHDR data starts at byte 16: width(4) height(4) depth(1) colortype(1).
        assert_eq!(png[24], 16, "bit depth");
        assert_eq!(png[25], ColorType::GrayscaleAlpha.code(), "colour type");
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
