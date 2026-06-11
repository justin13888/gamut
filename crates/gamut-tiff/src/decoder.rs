//! The TIFF decoder.

use gamut_core::{Decoder, Dimensions, Error, Result};

use crate::compression::{Compression, packbits};
use crate::ifd::{Ifd, PhotometricInterpretation};
use crate::{reader, tags};

/// Decoder for uncompressed baseline TIFF images.
///
/// This phase reads `Compression = None` (1), chunky, 8-bit grayscale and RGB images. Other
/// compressions and colour modes return [`Error::Unsupported`] until their phases land.
#[derive(Debug, Clone, Default)]
pub struct TiffDecoder {
    _private: (),
}

/// An image decoded to interleaved 8-bit samples in `BlackIsZero`/RGB convention.
struct DecodedImage {
    dims: Dimensions,
    samples_per_pixel: usize,
    pixels: Vec<u8>,
}

impl TiffDecoder {
    /// Creates a decoder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Decodes the first image to interleaved 8-bit RGB (grayscale is replicated across channels).
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] for malformed input, or [`Error::Unsupported`] for a
    /// feature not yet implemented.
    pub fn decode_to_rgb8(&self, data: &[u8], out: &mut Vec<u8>) -> Result<Dimensions> {
        let img = decode_image(data)?;
        match img.samples_per_pixel {
            1 => {
                out.reserve(img.pixels.len() * 3);
                for &v in &img.pixels {
                    out.extend_from_slice(&[v, v, v]);
                }
            }
            3 => out.extend_from_slice(&img.pixels),
            _ => {
                return Err(Error::Unsupported(
                    "TIFF: cannot present this sample layout as RGB",
                ));
            }
        }
        Ok(img.dims)
    }

    /// Decodes the first image to 8-bit grayscale; errors unless it is a single-sample image.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] for malformed input, or [`Error::Unsupported`] if the image
    /// is not single-sample grayscale.
    pub fn decode_to_gray8(&self, data: &[u8], out: &mut Vec<u8>) -> Result<Dimensions> {
        let img = decode_image(data)?;
        if img.samples_per_pixel != 1 {
            return Err(Error::Unsupported("TIFF: image is not grayscale"));
        }
        out.extend_from_slice(&img.pixels);
        Ok(img.dims)
    }
}

impl Decoder for TiffDecoder {
    fn decode(&self, data: &[u8], out: &mut Vec<u8>) -> Result<Dimensions> {
        self.decode_to_rgb8(data, out)
    }
}

/// Reads a required unsigned-integer tag.
fn require_u32(ifd: &Ifd, tag: u16, what: &'static str) -> Result<u32> {
    ifd.get_u32(tag).ok_or(Error::InvalidInput(what))
}

fn decode_image(data: &[u8]) -> Result<DecodedImage> {
    let file = reader::read(data)?;
    let ifd = &file.ifds[0];

    let width = require_u32(ifd, tags::IMAGE_WIDTH, "TIFF: missing ImageWidth")? as usize;
    let height = require_u32(ifd, tags::IMAGE_LENGTH, "TIFF: missing ImageLength")? as usize;
    if width == 0 || height == 0 {
        return Err(Error::InvalidInput("TIFF: zero-sized image"));
    }

    let compression = Compression::from_code(ifd.get_u32(tags::COMPRESSION).unwrap_or(1))
        .ok_or(Error::Unsupported("TIFF: unknown compression"))?;
    if !matches!(compression, Compression::None | Compression::PackBits) {
        return Err(Error::Unsupported("TIFF: compression not supported yet"));
    }
    if ifd.get_u32(tags::PLANAR_CONFIGURATION).unwrap_or(1) != 1 {
        return Err(Error::Unsupported(
            "TIFF: planar configuration not supported yet",
        ));
    }

    if ifd.get_u32(tags::FILL_ORDER).unwrap_or(1) != 1 {
        return Err(Error::Unsupported("TIFF: FillOrder 2 not supported"));
    }
    let spp = ifd.get_u32(tags::SAMPLES_PER_PIXEL).unwrap_or(1) as usize;
    let bits = ifd
        .get_u32_vec(tags::BITS_PER_SAMPLE)
        .unwrap_or_else(|| vec![1; spp]);
    if bits.len() != spp || bits.iter().any(|&b| b != bits[0]) {
        return Err(Error::Unsupported("TIFF: mixed bit depths not supported"));
    }
    let bps = bits[0];

    let photometric = PhotometricInterpretation::from_code(require_u32(
        ifd,
        tags::PHOTOMETRIC_INTERPRETATION,
        "TIFF: missing PhotometricInterpretation",
    )?)
    .ok_or(Error::Unsupported(
        "TIFF: unknown photometric interpretation",
    ))?;
    // `white_is_zero` selects which sample value maps to white (TIFF 6.0 §8 PhotometricInterpretation).
    let white_is_zero = match (spp, bps, photometric) {
        (1, 1 | 8, PhotometricInterpretation::WhiteIsZero) => true,
        (1, 1 | 8, PhotometricInterpretation::BlackIsZero) => false,
        (3, 8, PhotometricInterpretation::Rgb) => false,
        _ => {
            return Err(Error::Unsupported(
                "TIFF: photometric/sample combination not supported yet",
            ));
        }
    };

    // Bytes of one stored (packed) row, before unpacking to 8-bit output samples.
    let stored_row_bytes = match bps {
        8 => width
            .checked_mul(spp)
            .ok_or(Error::InvalidInput("TIFF: image too large"))?,
        1 => width.div_ceil(8), // spp == 1, guaranteed by the match above
        _ => {
            return Err(Error::Unsupported(
                "TIFF: only 1- and 8-bit samples supported so far",
            ));
        }
    };
    let stored_total = stored_row_bytes
        .checked_mul(height)
        .ok_or(Error::InvalidInput("TIFF: image too large"))?;

    let rows_per_strip = match ifd.get_u32(tags::ROWS_PER_STRIP) {
        Some(0) | None => height, // default: a single strip
        Some(r) => (r as usize).min(height),
    };
    let offsets = ifd
        .get_u32_vec(tags::STRIP_OFFSETS)
        .ok_or(Error::InvalidInput("TIFF: missing StripOffsets"))?;
    let counts = ifd
        .get_u32_vec(tags::STRIP_BYTE_COUNTS)
        .ok_or(Error::InvalidInput("TIFF: missing StripByteCounts"))?;
    let strips = height.div_ceil(rows_per_strip);
    if offsets.len() != strips || counts.len() != strips {
        return Err(Error::InvalidInput("TIFF: strip count mismatch"));
    }

    // Decompress strips into the packed (stored) row bytes.
    let mut packed = Vec::with_capacity(stored_total);
    for (i, (&off, &cnt)) in offsets.iter().zip(&counts).enumerate() {
        let rows = rows_per_strip.min(height - i * rows_per_strip);
        let want = rows * stored_row_bytes;
        let raw = data
            .get(off as usize..off as usize + cnt as usize)
            .ok_or(Error::InvalidInput("TIFF: strip out of bounds"))?;
        match compression {
            Compression::PackBits => packed.extend_from_slice(&packbits::decode(raw, want)?),
            _ => {
                let strip = raw
                    .get(..want)
                    .ok_or(Error::InvalidInput("TIFF: strip shorter than expected"))?;
                packed.extend_from_slice(strip);
            }
        }
    }
    debug_assert_eq!(packed.len(), stored_total);

    // Unpack the stored bytes into 8-bit output samples.
    let (out_spp, pixels) = if bps == 8 {
        let mut px = packed;
        if white_is_zero {
            for v in &mut px {
                *v = 255 - *v;
            }
        }
        (spp, px)
    } else {
        let mut px = Vec::with_capacity(width * height);
        for y in 0..height {
            let row = &packed[y * stored_row_bytes..(y + 1) * stored_row_bytes];
            for x in 0..width {
                let bit = (row[x / 8] >> (7 - (x % 8))) & 1;
                let white = if white_is_zero { bit == 0 } else { bit == 1 };
                px.push(if white { 255 } else { 0 });
            }
        }
        (1, px)
    };

    Ok(DecodedImage {
        dims: Dimensions {
            width: width as u32,
            height: height as u32,
        },
        samples_per_pixel: out_spp,
        pixels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encoder::TiffEncoder;
    use crate::ifd::ByteOrder;

    #[test]
    fn rejects_truncated_file() {
        let dec = TiffDecoder::new();
        let mut out = Vec::new();
        assert!(dec.decode_to_rgb8(&[], &mut out).is_err());
    }

    #[test]
    fn gray_roundtrips_both_orders() {
        for order in [ByteOrder::LittleEndian, ByteOrder::BigEndian] {
            let dims = Dimensions {
                width: 5,
                height: 3,
            };
            let pixels: Vec<u8> = (0..15).collect();
            let mut tiff = Vec::new();
            TiffEncoder::new()
                .with_byte_order(order)
                .encode_gray8(&pixels, dims, &mut tiff)
                .expect("encode");
            let mut out = Vec::new();
            let got = TiffDecoder::new()
                .decode_to_gray8(&tiff, &mut out)
                .expect("decode");
            assert_eq!((got.width, got.height), (5, 3));
            assert_eq!(out, pixels);
        }
    }
}
