//! The TIFF decoder.

use gamut_core::{Decoder, Dimensions, Error, Result};

use crate::compression::{Compression, ccitt, lzw, packbits, predictor};
use crate::ifd::{Ifd, PhotometricInterpretation};
use crate::{reader, tags};

/// Decoder for baseline TIFF images.
///
/// Reads chunky images compressed with `None` or PackBits: 8-bit grayscale/RGB, 1-bit bilevel,
/// and 8-bit palette colour. Other compressions and colour modes return [`Error::Unsupported`]
/// until their phases land.
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

/// How a decoded image's stored samples map to output pixels.
enum Mode {
    /// Grayscale; `white_is_zero` selects which sample value is white.
    Gray { white_is_zero: bool },
    /// Interleaved RGB.
    Rgb,
    /// Interleaved RGBA (RGB + one extra alpha sample).
    Rgba,
    /// Interleaved CMYK (4 separated ink samples).
    Cmyk,
    /// Palette colour: 8-bit indices into a 3×256 16-bit `ColorMap`.
    Palette(Vec<u32>),
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
            4 => {
                for px in img.pixels.chunks_exact(4) {
                    out.extend_from_slice(&px[0..3]); // drop alpha
                }
            }
            _ => {
                return Err(Error::Unsupported(
                    "TIFF: cannot present this sample layout as RGB",
                ));
            }
        }
        Ok(img.dims)
    }

    /// Decodes the first image to interleaved 8-bit RGBA (RGB gains opaque alpha, grayscale is
    /// replicated then made opaque).
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] for malformed input, or [`Error::Unsupported`] for a
    /// feature not yet implemented.
    pub fn decode_to_rgba8(&self, data: &[u8], out: &mut Vec<u8>) -> Result<Dimensions> {
        let img = decode_image(data)?;
        match img.samples_per_pixel {
            1 => {
                for &v in &img.pixels {
                    out.extend_from_slice(&[v, v, v, 255]);
                }
            }
            3 => {
                for px in img.pixels.chunks_exact(3) {
                    out.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
            }
            4 => out.extend_from_slice(&img.pixels),
            _ => {
                return Err(Error::Unsupported(
                    "TIFF: cannot present this sample layout as RGBA",
                ));
            }
        }
        Ok(img.dims)
    }

    /// Decodes the first image to interleaved 8-bit CMYK; errors unless it is a 4-sample image.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] for malformed input, or [`Error::Unsupported`] if the image
    /// is not 4-sample (CMYK).
    pub fn decode_to_cmyk8(&self, data: &[u8], out: &mut Vec<u8>) -> Result<Dimensions> {
        let img = decode_image(data)?;
        if img.samples_per_pixel != 4 {
            return Err(Error::Unsupported("TIFF: image is not 4-sample CMYK"));
        }
        out.extend_from_slice(&img.pixels);
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
    if !matches!(
        compression,
        Compression::None
            | Compression::PackBits
            | Compression::CcittRle
            | Compression::CcittGroup4Fax
            | Compression::Lzw
    ) {
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
    if matches!(
        compression,
        Compression::CcittRle | Compression::CcittGroup4Fax
    ) && bps != 1
    {
        return Err(Error::Unsupported(
            "TIFF: CCITT coding requires a bilevel image",
        ));
    }
    let use_predictor = match ifd.get_u32(tags::PREDICTOR).unwrap_or(1) {
        1 => false,
        2 => true,
        _ => return Err(Error::Unsupported("TIFF: unknown predictor")),
    };
    if use_predictor && bps != 8 {
        return Err(Error::Unsupported("TIFF: predictor requires 8-bit samples"));
    }

    let photometric = PhotometricInterpretation::from_code(require_u32(
        ifd,
        tags::PHOTOMETRIC_INTERPRETATION,
        "TIFF: missing PhotometricInterpretation",
    )?)
    .ok_or(Error::Unsupported(
        "TIFF: unknown photometric interpretation",
    ))?;
    // How stored samples become the decoded output (TIFF 6.0 §8 PhotometricInterpretation).
    let mode = match (spp, bps, photometric) {
        (1, 1 | 8, PhotometricInterpretation::WhiteIsZero) => Mode::Gray {
            white_is_zero: true,
        },
        (1, 1 | 8, PhotometricInterpretation::BlackIsZero) => Mode::Gray {
            white_is_zero: false,
        },
        (3, 8, PhotometricInterpretation::Rgb) => Mode::Rgb,
        (4, 8, PhotometricInterpretation::Rgb) => Mode::Rgba,
        (4, 8, PhotometricInterpretation::Cmyk) => Mode::Cmyk,
        (1, 8, PhotometricInterpretation::Palette) => {
            let cm = ifd
                .get_u32_vec(tags::COLOR_MAP)
                .ok_or(Error::InvalidInput("TIFF: palette image missing ColorMap"))?;
            if cm.len() != 3 * 256 {
                return Err(Error::InvalidInput(
                    "TIFF: ColorMap must have 3*256 entries",
                ));
            }
            Mode::Palette(cm)
        }
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

    // Reassemble the stored (packed) row bytes from tiles or strips.
    let layout = Layout {
        width,
        height,
        spp,
        bps,
        stored_row_bytes,
        compression,
    };
    let mut packed = if ifd.get(tags::TILE_WIDTH).is_some() {
        decode_tiles(ifd, data, &layout)?
    } else {
        decode_strips(ifd, data, &layout)?
    };
    debug_assert_eq!(packed.len(), stored_total);

    // Reverse the horizontal-differencing predictor (8-bit only) before unpacking.
    if use_predictor {
        predictor::reverse(&mut packed, stored_row_bytes, spp);
    }

    // Unpack the stored bytes into 8-bit output samples per the photometric mode.
    let (out_spp, pixels) = match mode {
        Mode::Rgb => (3, packed),
        Mode::Rgba | Mode::Cmyk => (4, packed),
        Mode::Gray { white_is_zero } if bps == 8 => {
            let mut px = packed;
            if white_is_zero {
                for v in &mut px {
                    *v = 255 - *v;
                }
            }
            (1, px)
        }
        Mode::Gray { white_is_zero } => {
            // bps == 1: expand each MSB-first bit to a 0/255 sample.
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
        }
        Mode::Palette(cm) => {
            // Each 8-bit index selects a 16-bit RGB triple; the high byte is the 8-bit sample.
            let mut px = Vec::with_capacity(width * height * 3);
            for &idx in &packed {
                let i = idx as usize;
                px.push((cm[i] >> 8) as u8);
                px.push((cm[256 + i] >> 8) as u8);
                px.push((cm[512 + i] >> 8) as u8);
            }
            (3, px)
        }
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

/// The decoded image's storage parameters, shared by the strip and tile readers.
struct Layout {
    width: usize,
    height: usize,
    spp: usize,
    bps: u32,
    stored_row_bytes: usize,
    compression: Compression,
}

/// Decompresses one strip/tile of byte-level data (`None`/PackBits/LZW) to `want` bytes.
fn decompress_simple(raw: &[u8], want: usize, compression: Compression) -> Result<Vec<u8>> {
    match compression {
        Compression::None => raw
            .get(..want)
            .map(<[u8]>::to_vec)
            .ok_or(Error::InvalidInput("TIFF: block shorter than expected")),
        Compression::PackBits => packbits::decode(raw, want),
        Compression::Lzw => lzw::decode(raw, want),
        _ => Err(Error::Unsupported(
            "TIFF: compression not supported for this layout",
        )),
    }
}

/// Reassembles the stored row bytes from strips.
fn decode_strips(ifd: &Ifd, data: &[u8], l: &Layout) -> Result<Vec<u8>> {
    let rows_per_strip = match ifd.get_u32(tags::ROWS_PER_STRIP) {
        Some(0) | None => l.height,
        Some(r) => (r as usize).min(l.height),
    };
    let offsets = ifd
        .get_u32_vec(tags::STRIP_OFFSETS)
        .ok_or(Error::InvalidInput("TIFF: missing StripOffsets"))?;
    let counts = ifd
        .get_u32_vec(tags::STRIP_BYTE_COUNTS)
        .ok_or(Error::InvalidInput("TIFF: missing StripByteCounts"))?;
    let strips = l.height.div_ceil(rows_per_strip);
    if offsets.len() != strips || counts.len() != strips {
        return Err(Error::InvalidInput("TIFF: strip count mismatch"));
    }
    let mut packed = Vec::with_capacity(l.stored_row_bytes * l.height);
    for (i, (&off, &cnt)) in offsets.iter().zip(&counts).enumerate() {
        let rows = rows_per_strip.min(l.height - i * rows_per_strip);
        let want = rows * l.stored_row_bytes;
        let raw = data
            .get(off as usize..off as usize + cnt as usize)
            .ok_or(Error::InvalidInput("TIFF: strip out of bounds"))?;
        match l.compression {
            Compression::CcittRle => {
                packed.extend_from_slice(&ccitt::mh_decode_strip(raw, rows, l.width)?);
            }
            Compression::CcittGroup4Fax => {
                packed.extend_from_slice(&ccitt::g4_decode_strip(raw, rows, l.width)?);
            }
            other => packed.extend_from_slice(&decompress_simple(raw, want, other)?),
        }
    }
    Ok(packed)
}

/// Reassembles the stored row bytes from tiles (8-bit only), cropping the edge-tile padding.
fn decode_tiles(ifd: &Ifd, data: &[u8], l: &Layout) -> Result<Vec<u8>> {
    if l.bps != 8 {
        return Err(Error::Unsupported(
            "TIFF: tiled images supported only for 8-bit samples so far",
        ));
    }
    let tw = ifd
        .get_u32(tags::TILE_WIDTH)
        .ok_or(Error::InvalidInput("TIFF: missing TileWidth"))? as usize;
    let th = ifd
        .get_u32(tags::TILE_LENGTH)
        .ok_or(Error::InvalidInput("TIFF: missing TileLength"))? as usize;
    if tw == 0 || th == 0 {
        return Err(Error::InvalidInput("TIFF: zero tile dimension"));
    }
    let offsets = ifd
        .get_u32_vec(tags::TILE_OFFSETS)
        .ok_or(Error::InvalidInput("TIFF: missing TileOffsets"))?;
    let counts = ifd
        .get_u32_vec(tags::TILE_BYTE_COUNTS)
        .ok_or(Error::InvalidInput("TIFF: missing TileByteCounts"))?;
    let across = l.width.div_ceil(tw);
    let down = l.height.div_ceil(th);
    if offsets.len() != across * down || counts.len() != across * down {
        return Err(Error::InvalidInput("TIFF: tile count mismatch"));
    }
    let tile_row_bytes = tw * l.spp;
    let tile_size = th * tile_row_bytes;
    let mut packed = vec![0u8; l.stored_row_bytes * l.height];
    for ty in 0..down {
        for tx in 0..across {
            let idx = ty * across + tx;
            let (off, cnt) = (offsets[idx] as usize, counts[idx] as usize);
            let raw = data
                .get(off..off + cnt)
                .ok_or(Error::InvalidInput("TIFF: tile out of bounds"))?;
            let tile = decompress_simple(raw, tile_size, l.compression)?;
            let copy_cols = tw.min(l.width - tx * tw);
            for r in 0..th {
                let dst_row = ty * th + r;
                if dst_row >= l.height {
                    break;
                }
                let src = r * tile_row_bytes;
                let dst = dst_row * l.stored_row_bytes + tx * tw * l.spp;
                packed[dst..dst + copy_cols * l.spp]
                    .copy_from_slice(&tile[src..src + copy_cols * l.spp]);
            }
        }
    }
    Ok(packed)
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
