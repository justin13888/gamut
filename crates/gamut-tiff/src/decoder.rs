//! The TIFF decoder.

use gamut_core::{Cmyk8, DecodeImage, Dimensions, Error, Gray8, ImageBuf, Result, Rgb8, Rgba8};

use crate::compression::{Compression, ccitt, lzw, packbits, predictor};
use crate::ifd::PhotometricInterpretation;
use crate::palette::Palette8;
use crate::tags;
use gamut_ifd::{Ifd, read};

/// Decoder for baseline TIFF images.
///
/// Reads chunky images compressed with `None` or PackBits: 8-bit grayscale/RGB, 1-bit bilevel,
/// and 8-bit palette colour. Other compressions and colour modes return [`Error::Unsupported`]
/// until their phases land.
#[derive(Debug, Clone, Default)]
pub struct TiffDecoder {
    _private: (),
}

/// Upper bound on a decoded image's stored bytes, guarding against malformed huge dimensions and
/// decompression bombs (64 MiB — e.g. a 4096×4096 RGBA image).
const MAX_IMAGE_BYTES: usize = 64 << 20;

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
    /// Palette colour: 8-bit indices into a [`Palette8`] colour table. Boxed because the 768-byte
    /// table would otherwise dwarf the other variants.
    Palette(Box<Palette8>),
}

impl TiffDecoder {
    /// Creates a decoder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of pages (subfile IFDs) in a TIFF.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if the file header or IFD chain is malformed.
    pub fn page_count(&self, data: &[u8]) -> Result<usize> {
        Ok(read(data)?.ifds.len())
    }

    /// Decodes page `page` of a multi-page TIFF to interleaved 8-bit [`Rgb8`] (page 0 is the first;
    /// grayscale is replicated across channels, any alpha is dropped). Multi-page access is
    /// TIFF-specific, so it stays inherent; the [`DecodeImage`] impls present page 0.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] for malformed input or an out-of-range page, or
    /// [`Error::Unsupported`] for a feature not yet implemented.
    pub fn decode_page(&self, data: &[u8], page: usize) -> Result<ImageBuf<Rgb8>> {
        let img = decode_page_samples(data, page)?;
        ImageBuf::new(present_rgb(&img)?, img.dims)
    }
}

impl DecodeImage<Rgb8> for TiffDecoder {
    /// Grayscale is replicated across channels; any alpha is dropped.
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<Rgb8>> {
        self.decode_page(data, 0)
    }
}

impl DecodeImage<Rgba8> for TiffDecoder {
    /// RGB gains opaque alpha; grayscale is replicated then made opaque.
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<Rgba8>> {
        let img = decode_page_samples(data, 0)?;
        ImageBuf::new(present_rgba(&img)?, img.dims)
    }
}

impl DecodeImage<Cmyk8> for TiffDecoder {
    /// Errors unless the image is 4-sample; the samples pass through unchanged.
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<Cmyk8>> {
        let img = decode_page_samples(data, 0)?;
        if img.samples_per_pixel != 4 {
            return Err(Error::Unsupported("TIFF: image is not 4-sample CMYK"));
        }
        ImageBuf::new(img.pixels, img.dims)
    }
}

impl DecodeImage<Gray8> for TiffDecoder {
    /// Errors unless the image is single-sample; the samples pass through unchanged.
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<Gray8>> {
        let img = decode_page_samples(data, 0)?;
        if img.samples_per_pixel != 1 {
            return Err(Error::Unsupported("TIFF: image is not grayscale"));
        }
        ImageBuf::new(img.pixels, img.dims)
    }
}

/// Presents decoded samples as interleaved 8-bit RGB (1 → replicated, 3 → as-is, 4 → alpha dropped).
fn present_rgb(img: &DecodedImage) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(img.dims.width as usize * img.dims.height as usize * 3);
    match img.samples_per_pixel {
        1 => {
            for &v in &img.pixels {
                out.extend_from_slice(&[v, v, v]);
            }
        }
        3 => out.extend_from_slice(&img.pixels),
        4 => {
            for px in img.pixels.chunks_exact(4) {
                out.extend_from_slice(&px[0..3]);
            }
        }
        _ => {
            return Err(Error::Unsupported(
                "TIFF: cannot present this sample layout as RGB",
            ));
        }
    }
    Ok(out)
}

/// Presents decoded samples as interleaved 8-bit RGBA (1 → replicated opaque, 3 → opaque, 4 → as-is).
fn present_rgba(img: &DecodedImage) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(img.dims.width as usize * img.dims.height as usize * 4);
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
    Ok(out)
}

/// Reads a required unsigned-integer tag.
fn require_u32(ifd: &Ifd, tag: u16, what: &'static str) -> Result<u32> {
    ifd.get_u32(tag).ok_or(Error::InvalidInput(what))
}

fn decode_page_samples(data: &[u8], page: usize) -> Result<DecodedImage> {
    let file = read(data)?;
    let ifd = file
        .ifds
        .get(page)
        .ok_or(Error::InvalidInput("TIFF: page index out of range"))?;

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
            Mode::Palette(Box::new(Palette8::from_tiff_colormap(&cm)?))
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
    if stored_total > MAX_IMAGE_BYTES {
        return Err(Error::Unsupported("TIFF: image exceeds the size limit"));
    }

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
        Mode::Palette(palette) => {
            // Each 8-bit index selects an RGB triple from the colour table.
            let mut px = Vec::with_capacity(width * height * 3);
            for &idx in &packed {
                px.extend_from_slice(&palette.entry(idx));
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
    let tile_row_bytes = tw
        .checked_mul(l.spp)
        .ok_or(Error::InvalidInput("TIFF: tile too large"))?;
    let tile_size = th
        .checked_mul(tile_row_bytes)
        .ok_or(Error::InvalidInput("TIFF: tile too large"))?;
    if tile_size > MAX_IMAGE_BYTES {
        return Err(Error::Unsupported("TIFF: tile exceeds the size limit"));
    }
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
    use gamut_core::{EncodeImage, ImageRef};
    use gamut_ifd::ByteOrder;

    #[test]
    fn rejects_truncated_file() {
        let dec = TiffDecoder::new();
        let got: Result<ImageBuf<Rgb8>> = dec.decode_image(&[]);
        assert!(got.is_err());
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
                .encode_image(ImageRef::<Gray8>::new(&pixels, dims).unwrap(), &mut tiff)
                .expect("encode");
            let got: ImageBuf<Gray8> = TiffDecoder::new().decode_image(&tiff).expect("decode");
            assert_eq!(got.dimensions(), dims);
            assert_eq!(got.as_samples(), pixels.as_slice());
        }
    }
}
