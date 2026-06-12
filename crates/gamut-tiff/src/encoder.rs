//! The TIFF encoder.

use gamut_core::{Dimensions, Encoder, Error, Result};

use crate::compression::{Compression, ccitt, lzw, packbits, predictor};
use crate::ifd::{ByteOrder, Ifd, PhotometricInterpretation, Predictor, Value};
use crate::{tags, writer};

/// The on-disk sample layout of an image, shared by the 8-bit and bilevel encode paths.
struct SampleLayout {
    spp: usize,
    bits_per_sample: u16,
    stored_row_bytes: usize,
    photometric: PhotometricInterpretation,
}

/// Encoder for baseline TIFF images.
///
/// Writes chunky (`PlanarConfiguration = 1`) strips, optionally PackBits-compressed
/// ([`Self::with_compression`]). Supports 8-bit grayscale/RGB and 1-bit bilevel; richer colour
/// modes and compression schemes are added in later phases.
#[derive(Debug, Clone)]
pub struct TiffEncoder {
    order: ByteOrder,
    compression: Compression,
    predictor: Predictor,
}

impl Default for TiffEncoder {
    fn default() -> Self {
        Self {
            order: ByteOrder::LittleEndian,
            compression: Compression::None,
            predictor: Predictor::None,
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
    #[must_use]
    pub fn with_compression(mut self, compression: Compression) -> Self {
        self.compression = compression;
        self
    }

    /// Returns a copy of this encoder that applies `predictor` before compression.
    ///
    /// [`Predictor::HorizontalDifferencing`] requires 8-bit samples and pairs well with LZW.
    #[must_use]
    pub fn with_predictor(mut self, predictor: Predictor) -> Self {
        self.predictor = predictor;
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
        self.encode_8bit(pixels, dims, 1, PhotometricInterpretation::BlackIsZero, out)
    }

    /// Encodes an 8-bit RGB image: three interleaved samples per pixel (`RGBRGB…`).
    ///
    /// `pixels` is `width * height * 3` bytes, row-major. Returns the number of bytes written.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `pixels` does not match `dims` or `dims` is empty.
    pub fn encode_rgb8(&self, pixels: &[u8], dims: Dimensions, out: &mut Vec<u8>) -> Result<usize> {
        self.encode_8bit(pixels, dims, 3, PhotometricInterpretation::Rgb, out)
    }

    /// Encodes a 1-bit bilevel image, stored as `BlackIsZero` (one bit per pixel, MSB-first).
    ///
    /// `pixels` is `width * height` bytes, one per pixel: zero is black, any non-zero value is
    /// white. Returns the number of bytes written.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `pixels` does not match `dims` or `dims` is empty.
    pub fn encode_bilevel(
        &self,
        pixels: &[u8],
        dims: Dimensions,
        out: &mut Vec<u8>,
    ) -> Result<usize> {
        let (w, h) = (dims.width as usize, dims.height as usize);
        if w == 0 || h == 0 {
            return Err(Error::InvalidInput("TIFF: zero-sized image"));
        }
        if pixels.len()
            != w.checked_mul(h)
                .ok_or(Error::InvalidInput("TIFF: image too large"))?
        {
            return Err(Error::InvalidInput(
                "TIFF: pixel buffer length does not match dimensions",
            ));
        }
        let stored_row_bytes = w.div_ceil(8);
        let mut packed = vec![0u8; stored_row_bytes * h];
        for y in 0..h {
            let row = &pixels[y * w..(y + 1) * w];
            let dst = &mut packed[y * stored_row_bytes..(y + 1) * stored_row_bytes];
            for (x, &p) in row.iter().enumerate() {
                if p != 0 {
                    dst[x / 8] |= 0x80 >> (x % 8);
                }
            }
        }
        self.encode_packed(
            &packed,
            dims,
            &SampleLayout {
                spp: 1,
                bits_per_sample: 1,
                stored_row_bytes,
                photometric: PhotometricInterpretation::BlackIsZero,
            },
            &[],
            out,
        )
    }

    /// Encodes an 8-bit palette-colour image.
    ///
    /// `indices` is `width * height` bytes (one palette index per pixel); `palette` is `256 * 3`
    /// bytes of 8-bit RGB (entry `i` is `palette[3*i..3*i+3]`). Returns the number of bytes written.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `indices` does not match `dims`, `dims` is empty, or
    /// `palette` is not exactly 256 RGB entries.
    pub fn encode_palette8(
        &self,
        indices: &[u8],
        palette: &[u8],
        dims: Dimensions,
        out: &mut Vec<u8>,
    ) -> Result<usize> {
        let (w, h) = (dims.width as usize, dims.height as usize);
        if w == 0 || h == 0 {
            return Err(Error::InvalidInput("TIFF: zero-sized image"));
        }
        if indices.len()
            != w.checked_mul(h)
                .ok_or(Error::InvalidInput("TIFF: image too large"))?
        {
            return Err(Error::InvalidInput(
                "TIFF: index buffer length does not match dimensions",
            ));
        }
        if palette.len() != 256 * 3 {
            return Err(Error::InvalidInput("TIFF: palette must be 256 RGB entries"));
        }
        // ColorMap: 3×256 16-bit values (all reds, then greens, then blues); 8-bit → 16-bit by ×257.
        let mut colormap = vec![0u16; 3 * 256];
        for i in 0..256 {
            colormap[i] = u16::from(palette[3 * i]) * 257;
            colormap[256 + i] = u16::from(palette[3 * i + 1]) * 257;
            colormap[512 + i] = u16::from(palette[3 * i + 2]) * 257;
        }
        self.encode_packed(
            indices,
            dims,
            &SampleLayout {
                spp: 1,
                bits_per_sample: 8,
                stored_row_bytes: w,
                photometric: PhotometricInterpretation::Palette,
            },
            &[(tags::COLOR_MAP, Value::Short(colormap))],
            out,
        )
    }

    fn encode_8bit(
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
        self.encode_packed(
            pixels,
            dims,
            &SampleLayout {
                spp,
                bits_per_sample: 8,
                stored_row_bytes: row_bytes,
                photometric,
            },
            &[],
            out,
        )
    }

    /// Lays out an image from already-packed sample bytes (`height * stored_row_bytes`), applying
    /// the strip codec and building the directory.
    fn encode_packed(
        &self,
        packed: &[u8],
        dims: Dimensions,
        layout: &SampleLayout,
        extra_fields: &[(u16, Value)],
        out: &mut Vec<u8>,
    ) -> Result<usize> {
        let h = dims.height as usize;
        let stored_row_bytes = layout.stored_row_bytes;

        // Apply the horizontal-differencing predictor (8-bit only) before compression.
        let predicting = self.predictor == Predictor::HorizontalDifferencing;
        if predicting && layout.bits_per_sample != 8 {
            return Err(Error::Unsupported("TIFF: predictor requires 8-bit samples"));
        }
        let predicted = predicting.then(|| {
            let mut buf = packed.to_vec();
            predictor::forward(&mut buf, stored_row_bytes, layout.spp);
            buf
        });
        let packed: &[u8] = predicted.as_deref().unwrap_or(packed);

        // Partition rows into strips of roughly 8 KB (TIFF 6.0 §7), then apply the strip codec.
        let rows_per_strip = (8192 / stored_row_bytes.max(1)).clamp(1, h);
        let mut strips: Vec<Vec<u8>> = Vec::new();
        let mut row = 0;
        while row < h {
            let rows = rows_per_strip.min(h - row);
            let start = row * stored_row_bytes;
            let raw = &packed[start..start + rows * stored_row_bytes];
            strips.push(self.compress_strip(raw, dims, layout)?);
            row += rows;
        }

        let mut ifd = Ifd::new();
        ifd.set(tags::IMAGE_WIDTH, dim_value(dims.width));
        ifd.set(tags::IMAGE_LENGTH, dim_value(dims.height));
        ifd.set(
            tags::BITS_PER_SAMPLE,
            Value::Short(vec![layout.bits_per_sample; layout.spp]),
        );
        ifd.set(
            tags::COMPRESSION,
            Value::Short(vec![self.compression.code()]),
        );
        ifd.set(
            tags::PHOTOMETRIC_INTERPRETATION,
            Value::Short(vec![layout.photometric.code()]),
        );
        ifd.set(
            tags::SAMPLES_PER_PIXEL,
            Value::Short(vec![layout.spp as u16]),
        );
        ifd.set(tags::ROWS_PER_STRIP, dim_value(rows_per_strip as u32));
        ifd.set(tags::X_RESOLUTION, Value::Rational(vec![(72, 1)]));
        ifd.set(tags::Y_RESOLUTION, Value::Rational(vec![(72, 1)]));
        ifd.set(tags::RESOLUTION_UNIT, Value::Short(vec![2])); // inch
        if predicting {
            ifd.set(tags::PREDICTOR, Value::Short(vec![2]));
        }
        for (tag, value) in extra_fields {
            ifd.set(*tag, value.clone());
        }

        let bytes = writer::write_image(self.order, &ifd, &strips);
        out.extend_from_slice(&bytes);
        Ok(bytes.len())
    }

    /// Applies the selected compression to one strip's already-packed bytes.
    fn compress_strip(
        &self,
        raw: &[u8],
        dims: Dimensions,
        layout: &SampleLayout,
    ) -> Result<Vec<u8>> {
        let row_bytes = layout.stored_row_bytes;
        match self.compression {
            Compression::None => Ok(raw.to_vec()),
            Compression::PackBits => {
                let mut out = Vec::new();
                for row in raw.chunks(row_bytes) {
                    packbits::encode_row(row, &mut out);
                }
                Ok(out)
            }
            Compression::CcittRle => {
                if layout.bits_per_sample != 1 {
                    return Err(Error::Unsupported(
                        "TIFF: Modified Huffman requires a bilevel image",
                    ));
                }
                ccitt::mh_encode_strip(raw, row_bytes, dims.width as usize)
            }
            Compression::CcittGroup4Fax => {
                if layout.bits_per_sample != 1 {
                    return Err(Error::Unsupported(
                        "TIFF: Group 4 fax requires a bilevel image",
                    ));
                }
                let rows = raw.len() / row_bytes;
                ccitt::g4_encode_strip(raw, row_bytes, rows, dims.width as usize)
            }
            Compression::Lzw => Ok(lzw::encode(raw)),
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
        assert!(enc.encode_bilevel(&[0; 3], dims, &mut out).is_err());
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
