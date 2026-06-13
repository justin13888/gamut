//! The PNG encoder: a [`PngEncoder`] builder implementing [`gamut_core::EncodeImage`] for each
//! supported pixel layout. This covers the four non-indexed colour types at 8- and 16-bit depth;
//! palette, sub-byte depths, ancillary chunks, and space optimisations layer on in later phases.

use gamut_core::{
    Bilevel, EncodeImage, Error, Gray8, Gray16, GrayAlpha8, GrayAlpha16, ImageRef, Indexed8, Pixel,
    Result, Rgb8, Rgb16, Rgba8, Rgba16,
};
use gamut_deflate::{DeflateEncoder, Level};

use crate::ancillary::{Ancillary, PhysicalUnit, SrgbIntent};
use crate::chunk::{self, SIGNATURE};
use crate::color::ColorType;
use crate::filter::{self, FilterStrategy};
use crate::ihdr;
use crate::pack;
use crate::palette::PngPalette;

/// IDAT payload cap. A decoder concatenates consecutive IDATs, so the split is transparent; a
/// large-ish cap keeps the 12-byte per-chunk overhead negligible.
const IDAT_MAX: usize = 1 << 16;

/// A reusable PNG encoder.
#[derive(Debug, Clone)]
pub struct PngEncoder {
    level: Level,
    filter: FilterStrategy,
    ancillary: Ancillary,
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
            ancillary: Ancillary::default(),
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

    /// Records an image gamma (gAMA chunk). `gamma` is the encoding gamma, e.g. `1.0 / 2.2`.
    #[must_use]
    pub fn with_gamma(mut self, gamma: f64) -> Self {
        self.ancillary.gamma = Some((gamma * 100_000.0).round().max(0.0) as u32);
        self
    }

    /// Records the standard colour-space rendering intent (sRGB chunk).
    #[must_use]
    pub fn with_srgb(mut self, intent: SrgbIntent) -> Self {
        self.ancillary.set_srgb(intent);
        self
    }

    /// Records the white point and RGB primary chromaticities (cHRM chunk), each as `(x, y)`.
    #[must_use]
    pub fn with_chromaticities(
        mut self,
        white: (f64, f64),
        red: (f64, f64),
        green: (f64, f64),
        blue: (f64, f64),
    ) -> Self {
        let q = |v: f64| (v * 100_000.0).round().max(0.0) as u32;
        self.ancillary.chrm = Some([
            q(white.0),
            q(white.1),
            q(red.0),
            q(red.1),
            q(green.0),
            q(green.1),
            q(blue.0),
            q(blue.1),
        ]);
        self
    }

    /// Records the number of significant bits per channel (sBIT chunk). The length must match the
    /// colour type (1 for grey, 2 for grey+alpha, 3 for RGB/indexed, 4 for RGBA).
    #[must_use]
    pub fn with_significant_bits(mut self, bits: &[u8]) -> Self {
        self.ancillary.sbit = Some(bits.to_vec());
        self
    }

    /// Records a greyscale background colour (bKGD chunk) for greyscale images.
    #[must_use]
    pub fn with_background_gray(mut self, gray: u16) -> Self {
        self.ancillary.bkgd = Some(gray.to_be_bytes().to_vec());
        self
    }

    /// Records an RGB background colour (bKGD chunk) for truecolour images.
    #[must_use]
    pub fn with_background_rgb(mut self, red: u16, green: u16, blue: u16) -> Self {
        let mut data = Vec::with_capacity(6);
        data.extend_from_slice(&red.to_be_bytes());
        data.extend_from_slice(&green.to_be_bytes());
        data.extend_from_slice(&blue.to_be_bytes());
        self.ancillary.bkgd = Some(data);
        self
    }

    /// Records a palette-index background colour (bKGD chunk) for indexed images.
    #[must_use]
    pub fn with_background_index(mut self, index: u8) -> Self {
        self.ancillary.bkgd = Some(vec![index]);
        self
    }

    /// Records the intended physical pixel dimensions (pHYs chunk).
    #[must_use]
    pub fn with_physical_dimensions(mut self, x_ppu: u32, y_ppu: u32, unit: PhysicalUnit) -> Self {
        self.ancillary.set_physical(x_ppu, y_ppu, unit);
        self
    }

    /// Records the last-modification time (tIME chunk), in UTC.
    #[must_use]
    pub fn with_time(
        mut self,
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    ) -> Self {
        self.ancillary
            .set_time(year, month, day, hour, minute, second);
        self
    }

    /// Adds an uncompressed Latin-1 text annotation (tEXt chunk).
    #[must_use]
    pub fn with_text(mut self, keyword: &str, text: &str) -> Self {
        self.ancillary.add_text_latin1(keyword, text);
        self
    }

    /// Adds a zlib-compressed Latin-1 text annotation (zTXt chunk).
    #[must_use]
    pub fn with_compressed_text(mut self, keyword: &str, text: &str) -> Self {
        self.ancillary.add_text_compressed(keyword, text);
        self
    }

    /// Adds an uncompressed UTF-8 text annotation (iTXt chunk).
    #[must_use]
    pub fn with_international_text(mut self, keyword: &str, text: &str) -> Self {
        self.ancillary.add_text_international(keyword, text);
        self
    }

    /// Embeds raw EXIF metadata (eXIf chunk). `exif` is the EXIF/TIFF byte stream beginning with the
    /// byte-order marker (`II`/`MM`) — for example the bytes produced by `gamut-exif`.
    #[must_use]
    pub fn with_exif(mut self, exif: &[u8]) -> Self {
        self.ancillary.exif = Some(exif.to_vec());
        self
    }

    /// Embeds an ICC colour profile (iCCP chunk), zlib-compressed. `profile` is the raw ICC profile
    /// — for example the bytes produced by `gamut-icc`. (Mutually exclusive with [`Self::with_srgb`]
    /// per the spec; set only one.)
    #[must_use]
    pub fn with_icc_profile(mut self, name: &str, profile: &[u8]) -> Self {
        self.ancillary.iccp = Some((name.to_string(), profile.to_vec()));
        self
    }

    /// Embeds an XMP packet (an iTXt chunk with the standard `XML:com.adobe.xmp` keyword). `xmp` is
    /// the XMP/RDF document — for example the bytes produced by `gamut-xmp`.
    #[must_use]
    pub fn with_xmp(mut self, xmp: &str) -> Self {
        self.ancillary
            .add_text_international("XML:com.adobe.xmp", xmp);
        self
    }

    /// Encodes an 8-bit indexed (palette) image. Indexed colour does not fit the single-buffer
    /// [`EncodeImage`] shape because it needs a separate palette, so it is an inherent method.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if any index is out of range for `palette`.
    pub fn encode_indexed8(
        &self,
        image: ImageRef<'_, Indexed8>,
        palette: &PngPalette,
        out: &mut Vec<u8>,
    ) -> Result<usize> {
        let indices = image.as_samples();
        let max_index = indices.iter().copied().max().unwrap_or(0);
        if usize::from(max_index) >= palette.len() {
            return Err(Error::InvalidInput("PNG: palette index out of range"));
        }
        let dims = image.dimensions();
        // Use the smallest bit depth that holds every index — a free, lossless space win.
        let depth = index_bit_depth(palette.len());
        let packed;
        let sample_bytes = if depth < 8 {
            packed =
                pack::pack_scanlines(indices, dims.width as usize, dims.height as usize, depth);
            packed.as_slice()
        } else {
            indices
        };
        let plte = palette.plte();
        let trns = palette.trns();
        Ok(self.write_png(
            (dims.width, dims.height),
            sample_bytes,
            ColorType::Indexed,
            depth,
            |out| {
                chunk::write_chunk(out, *b"PLTE", &plte);
                if let Some(alpha) = trns {
                    chunk::write_chunk(out, *b"tRNS", alpha);
                }
            },
            out,
        ))
    }

    /// Encodes an 8-bit-per-sample image (samples are already PNG's storage bytes).
    fn encode_8bit<P: Pixel<Sample = u8>>(
        &self,
        image: ImageRef<'_, P>,
        color: ColorType,
        out: &mut Vec<u8>,
    ) -> usize {
        let dims = image.dimensions();
        self.write_png(
            (dims.width, dims.height),
            image.as_samples(),
            color,
            8,
            |_| {},
            out,
        )
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
        self.write_png((dims.width, dims.height), &bytes, color, 16, |_| {}, out)
    }

    /// Shared back end: signature → IHDR → `pre_idat` chunks (e.g. PLTE/tRNS) → filtered +
    /// DEFLATE-compressed scanlines as IDAT(s) → IEND. `sample_bytes` is the image in PNG storage
    /// order; the stride is derived from `color` and `bit_depth`.
    fn write_png<F: FnOnce(&mut Vec<u8>)>(
        &self,
        (width, height): (u32, u32),
        sample_bytes: &[u8],
        color: ColorType,
        bit_depth: u8,
        pre_idat: F,
        out: &mut Vec<u8>,
    ) -> usize {
        // Stride in bytes per pixel (≥1, even for sub-byte depths) and the padded row length.
        let bits_per_pixel = color.channels() * bit_depth as usize;
        let bpp = bits_per_pixel.div_ceil(8).max(1);
        let row_bytes = (width as usize * bits_per_pixel).div_ceil(8);

        let start = out.len();
        out.extend_from_slice(&SIGNATURE);
        ihdr::write(out, width, height, bit_depth, color);
        self.ancillary.write_pre_plte(out); // colour-space chunks precede PLTE
        pre_idat(out); // PLTE + tRNS (indexed only)
        self.ancillary.write_post_plte(out); // background / physical / timing / text

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

/// The smallest indexed bit depth (1, 2, 4, or 8) that can address `palette_len` entries.
fn index_bit_depth(palette_len: usize) -> u8 {
    match palette_len {
        0..=2 => 1,
        3..=4 => 2,
        5..=16 => 4,
        _ => 8,
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
impl EncodeImage<Bilevel> for PngEncoder {
    /// Bilevel pixels (0 = black, non-zero = white) are packed to a 1-bit greyscale image.
    fn encode_image(&self, image: ImageRef<'_, Bilevel>, out: &mut Vec<u8>) -> Result<usize> {
        let dims = image.dimensions();
        let bits: Vec<u8> = image
            .as_samples()
            .iter()
            .map(|&v| u8::from(v != 0))
            .collect();
        let packed = pack::pack_scanlines(&bits, dims.width as usize, dims.height as usize, 1);
        Ok(self.write_png(
            (dims.width, dims.height),
            &packed,
            ColorType::Grayscale,
            1,
            |_| {},
            out,
        ))
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
