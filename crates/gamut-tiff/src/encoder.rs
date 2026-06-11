//! The TIFF encoder.

use gamut_core::{Dimensions, Encoder, Error, Result};

use crate::compression::{Compression, packbits};
use crate::ifd::{ByteOrder, Ifd, PhotometricInterpretation, Value};
use crate::{tags, writer};

/// Encoder for baseline TIFF images.
///
/// Writes chunky (`PlanarConfiguration = 1`) strips of 8-bit samples, optionally PackBits-compressed
/// ([`Self::with_compression`]). Richer colour modes and compression schemes are added in later
/// phases.
#[derive(Debug, Clone)]
pub struct TiffEncoder {
    order: ByteOrder,
    compression: Compression,
}

impl Default for TiffEncoder {
    fn default() -> Self {
        Self {
            order: ByteOrder::LittleEndian,
            compression: Compression::None,
        }
    }
}

impl TiffEncoder {
    /// Creates an encoder that writes little-endian (`II`) TIFF.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a copy of this encoder that writes in the given byte order.
    #[must_use]
    pub fn with_byte_order(mut self, order: ByteOrder) -> Self {
        self.order = order;
        self
    }

    /// Returns a copy of this encoder that compresses image data with `compression`.
    ///
    /// Supported so far: [`Compression::None`] and [`Compression::PackBits`].
    #[must_use]
    pub fn with_compression(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    /// Encodes an 8-bit grayscale image: one sample per pixel, `BlackIsZero`.
    ///
    /// `pixels` is `width * height` bytes, row-major. Returns the number of bytes written.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `pixels` does not match `dims` or `dims` is empty.
    pub fn encode_gray8(
        &self,
        pixels: &[u8],
        dims: Dimensions,
        out: &mut Vec<u8>,
    ) -> Result<usize> {
        self.encode_samples(pixels, dims, 1, PhotometricInterpretation::BlackIsZero, out)
    }

    /// Encodes an 8-bit RGB image: three interleaved samples per pixel (`RGBRGB…`).
    ///
    /// `pixels` is `width * height * 3` bytes, row-major. Returns the number of bytes written.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `pixels` does not match `dims` or `dims` is empty.
    pub fn encode_rgb8(&self, pixels: &[u8], dims: Dimensions, out: &mut Vec<u8>) -> Result<usize> {
        self.encode_samples(pixels, dims, 3, PhotometricInterpretation::Rgb, out)
    }

    fn encode_samples(
        &self,
        pixels: &[u8],
        dims: Dimensions,
        spp: usize,
        photometric: PhotometricInterpretation,
        out: &mut Vec<u8>,
    ) -> Result<usize> {
        let (w, h) = (dims.width as usize, dims.height as usize);
        if w == 0 || h == 0 {
            return Err(Error::InvalidInput("TIFF: zero-sized image"));
        }
        let row_bytes = w
            .checked_mul(spp)
            .ok_or(Error::InvalidInput("TIFF: image too large"))?;
        let expected = row_bytes
            .checked_mul(h)
            .ok_or(Error::InvalidInput("TIFF: image too large"))?;
        if pixels.len() != expected {
            return Err(Error::InvalidInput(
                "TIFF: pixel buffer length does not match dimensions",
            ));
        }

        // Partition rows into strips of roughly 8 KB (TIFF 6.0 §7 recommendation), then apply the
        // per-row strip codec.
        let rows_per_strip = (8192 / row_bytes).clamp(1, h);
        let mut strips: Vec<Vec<u8>> = Vec::new();
        let mut row = 0;
        while row < h {
            let rows = rows_per_strip.min(h - row);
            let start = row * row_bytes;
            let raw = &pixels[start..start + rows * row_bytes];
            strips.push(self.compress_strip(raw, row_bytes)?);
            row += rows;
        }

        let mut ifd = Ifd::new();
        ifd.set(tags::IMAGE_WIDTH, dim_value(dims.width));
        ifd.set(tags::IMAGE_LENGTH, dim_value(dims.height));
        ifd.set(tags::BITS_PER_SAMPLE, Value::Short(vec![8; spp]));
        ifd.set(
            tags::COMPRESSION,
            Value::Short(vec![self.compression.code()]),
        );
        ifd.set(
            tags::PHOTOMETRIC_INTERPRETATION,
            Value::Short(vec![photometric.code()]),
        );
        ifd.set(tags::SAMPLES_PER_PIXEL, Value::Short(vec![spp as u16]));
        ifd.set(tags::ROWS_PER_STRIP, dim_value(rows_per_strip as u32));
        ifd.set(tags::X_RESOLUTION, Value::Rational(vec![(72, 1)]));
        ifd.set(tags::Y_RESOLUTION, Value::Rational(vec![(72, 1)]));
        ifd.set(tags::RESOLUTION_UNIT, Value::Short(vec![2])); // inch

        let bytes = writer::write_image(self.order, &ifd, &strips);
        out.extend_from_slice(&bytes);
        Ok(bytes.len())
    }

    /// Applies the selected compression to one strip's raw bytes, row by row.
    fn compress_strip(&self, raw: &[u8], row_bytes: usize) -> Result<Vec<u8>> {
        match self.compression {
            Compression::None => Ok(raw.to_vec()),
            Compression::PackBits => {
                let mut out = Vec::new();
                for row in raw.chunks(row_bytes) {
                    packbits::encode_row(row, &mut out);
                }
                Ok(out)
            }
            _ => Err(Error::Unsupported(
                "TIFF: unsupported compression for encoding",
            )),
        }
    }
}

impl Encoder for TiffEncoder {
    fn encode(&self, pixels: &[u8], dims: Dimensions, out: &mut Vec<u8>) -> Result<usize> {
        self.encode_rgb8(pixels, dims, out)
    }
}

/// Stores a dimension/count as `SHORT` when it fits, else `LONG` (both are valid per §2).
fn dim_value(n: u32) -> Value {
    if n <= u32::from(u16::MAX) {
        Value::Short(vec![n as u16])
    } else {
        Value::Long(vec![n])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_mismatched_buffer() {
        let enc = TiffEncoder::new();
        let mut out = Vec::new();
        let dims = Dimensions {
            width: 2,
            height: 2,
        };
        assert!(enc.encode_rgb8(&[0; 11], dims, &mut out).is_err());
        assert!(enc.encode_gray8(&[0; 3], dims, &mut out).is_err());
        assert!(
            enc.encode_rgb8(
                &[],
                Dimensions {
                    width: 0,
                    height: 1
                },
                &mut out
            )
            .is_err()
        );
    }

    #[test]
    fn writes_a_well_formed_header() {
        let enc = TiffEncoder::new();
        let mut out = Vec::new();
        let n = enc
            .encode_rgb8(
                &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12],
                Dimensions {
                    width: 2,
                    height: 2,
                },
                &mut out,
            )
            .expect("encode");
        assert_eq!(n, out.len());
        assert_eq!(&out[0..2], b"II");
    }
}
