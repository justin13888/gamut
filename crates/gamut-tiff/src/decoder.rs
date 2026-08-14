//! The TIFF decoder.
//!
//! Decoding proper produces the image in the layout the file natively carries; presenting it as a
//! caller's chosen [`gamut_core::Pixel`] is then a pure conversion, delegated wholesale to
//! [`gamut_core::convert`] so TIFF applies exactly the same widening and narrowing rules as every
//! other gamut decoder. [`TiffDecoder::convert_policy`] selects which lossy conversions are
//! permitted; the default permits none.

use gamut_core::convert::{ConvertPolicy, RawImage, convert_from_raw};
use gamut_core::{
    Cmyk8, DecodeImage, Dimensions, Error, Gray8, ImageBuf, Pixel, PixelFormat, Result, Rgb8, Rgba8,
};
use gamut_ifd::{Ifd, read};

use crate::compression::{Compression, ccitt, deflate, lzw, packbits, predictor};
use crate::ifd::{PhotometricInterpretation, Predictor};
use crate::palette::Palette8;
use crate::tags;

/// Decoder for baseline TIFF images.
///
/// Reads chunky strips or tiles compressed with None, PackBits, LZW, Adobe Deflate, Modified
/// Huffman, or Group 4 fax. Supported layouts are 8-bit grayscale/RGB/RGBA/CMYK/palette and 1-bit
/// bilevel; other compression and colour modes return [`Error::Unsupported`].
///
/// A typed decode presents the image losslessly by default, so requesting a layout that cannot
/// hold the file — [`Rgb8`] for an RGBA TIFF, [`Gray8`] for a colour one — is refused rather than
/// silently narrowed. Call [`TiffDecoder::convert_policy`] to opt into the loss.
#[derive(Debug, Clone, Default)]
pub struct TiffDecoder {
    policy: ConvertPolicy,
}

/// Upper bound on a decoded image's stored bytes, guarding against malformed huge dimensions and
/// decompression bombs (64 MiB — e.g. a 4096×4096 RGBA image).
const MAX_IMAGE_BYTES: usize = 64 << 20;

/// An image decoded to interleaved 8-bit samples in `BlackIsZero`/RGB convention.
///
/// `format` is the layout those samples are *already* in — palette indices are expanded and
/// `WhiteIsZero` is inverted during decode, so what reaches [`gamut_core::convert`] is a plain
/// grayscale, RGB, RGBA, or CMYK buffer.
struct DecodedImage {
    dims: Dimensions,
    format: PixelFormat,
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

    /// Selects which lossy conversions a typed decode may perform.
    ///
    /// Defaults to [`ConvertPolicy::lossless`], under which a layout that cannot hold the file
    /// exactly is refused. Pass [`ConvertPolicy::permissive`] for the "just give me RGB" behaviour
    /// an application usually wants:
    ///
    /// ```no_run
    /// use gamut_core::{convert::ConvertPolicy, DecodeImage, ImageBuf, Rgb8};
    /// use gamut_tiff::TiffDecoder;
    ///
    /// # fn main() -> gamut_core::Result<()> {
    /// # let bytes: &[u8] = &[];
    /// let decoder = TiffDecoder::new().convert_policy(ConvertPolicy::permissive());
    /// let rgb: ImageBuf<Rgb8> = decoder.decode_image(bytes)?;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn convert_policy(mut self, policy: ConvertPolicy) -> Self {
        self.policy = policy;
        self
    }

    /// Decodes page `page` of a multi-page TIFF to interleaved 8-bit [`Rgb8`] (page 0 is the
    /// first). Multi-page access is TIFF-specific, so it stays inherent; the [`DecodeImage`] impls
    /// present page 0.
    ///
    /// Shorthand for [`TiffDecoder::decode_page_as`] at [`Rgb8`], and subject to the same
    /// [`ConvertPolicy`]: an RGBA or CMYK page is refused unless the policy permits the loss.
    ///
    /// # Errors
    ///
    /// As [`TiffDecoder::decode_page_as`].
    pub fn decode_page(&self, data: &[u8], page: usize) -> Result<ImageBuf<Rgb8>> {
        self.decode_page_as(data, page)
    }

    /// Decodes page `page` of a multi-page TIFF and presents it as pixel layout `P`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] for malformed input or an out-of-range page,
    /// [`Error::Unsupported`] for a feature not yet implemented, or [`Error::Unsupported`] for a
    /// conversion to `P` that would lose information this decoder's [`ConvertPolicy`] does not
    /// permit.
    pub fn decode_page_as<P: Pixel<Sample = u8>>(
        &self,
        data: &[u8],
        page: usize,
    ) -> Result<ImageBuf<P>> {
        let img = decode_page_samples(data, page)?;
        let raw = RawImage::new(&img.pixels, img.format, img.dims)?;
        convert_from_raw(raw, self.policy)
    }
}

impl DecodeImage<Rgb8> for TiffDecoder {
    /// Grayscale and palette images widen into RGB. An RGBA file needs an
    /// [`AlphaPolicy`](gamut_core::convert::AlphaPolicy), and a CMYK file cannot be presented as
    /// RGB at all (decode it as [`Cmyk8`]).
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<Rgb8>> {
        self.decode_page_as(data, 0)
    }
}

impl DecodeImage<Rgba8> for TiffDecoder {
    /// RGB gains opaque alpha; grayscale is replicated then made opaque.
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<Rgba8>> {
        self.decode_page_as(data, 0)
    }
}

impl DecodeImage<Cmyk8> for TiffDecoder {
    /// Errors unless the image is CMYK; the samples pass through unchanged. CMYK is an ink space,
    /// not a rearrangement of RGB, so no policy converts into or out of it.
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<Cmyk8>> {
        self.decode_page_as(data, 0)
    }
}

impl DecodeImage<Gray8> for TiffDecoder {
    /// A grayscale file passes through unchanged. A colour file needs a
    /// [`LumaPolicy`](gamut_core::convert::LumaPolicy) to be reduced to luma.
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<Gray8>> {
        self.decode_page_as(data, 0)
    }
}

/// Reads a required unsigned-integer tag.
fn require_u32(ifd: &Ifd, tag: u16, what: &'static str) -> Result<u32> {
    ifd.get_u32(tag)
        .ok_or_else(|| Error::invalid_input(env!("CARGO_PKG_NAME"), what))
}

fn decode_page_samples(data: &[u8], page: usize) -> Result<DecodedImage> {
    let file = read(data)?;
    let ifd = file.ifds.get(page).ok_or_else(|| {
        Error::invalid_input(env!("CARGO_PKG_NAME"), "TIFF: page index out of range")
    })?;

    let width = require_u32(ifd, tags::IMAGE_WIDTH, "TIFF: missing ImageWidth")? as usize;
    let height = require_u32(ifd, tags::IMAGE_LENGTH, "TIFF: missing ImageLength")? as usize;
    if width == 0 || height == 0 {
        return Err(Error::invalid_input(
            env!("CARGO_PKG_NAME"),
            "TIFF: zero-sized image",
        ));
    }

    let compression = Compression::try_from(ifd.get_u32(tags::COMPRESSION).unwrap_or(1))?;
    if !matches!(
        compression,
        Compression::None
            | Compression::PackBits
            | Compression::CcittRle
            | Compression::CcittGroup4Fax
            | Compression::Lzw
            | Compression::Deflate
    ) {
        return Err(Error::unsupported(
            env!("CARGO_PKG_NAME"),
            "TIFF: compression not supported yet",
        ));
    }
    if ifd.get_u32(tags::PLANAR_CONFIGURATION).unwrap_or(1) != 1 {
        return Err(Error::unsupported(
            env!("CARGO_PKG_NAME"),
            "TIFF: planar configuration not supported yet",
        ));
    }

    if ifd.get_u32(tags::FILL_ORDER).unwrap_or(1) != 1 {
        return Err(Error::unsupported(
            env!("CARGO_PKG_NAME"),
            "TIFF: FillOrder 2 not supported",
        ));
    }
    let spp = ifd.get_u32(tags::SAMPLES_PER_PIXEL).unwrap_or(1) as usize;
    let bits = ifd
        .get_u32_vec(tags::BITS_PER_SAMPLE)
        .unwrap_or_else(|| vec![1; spp]);
    if bits.len() != spp || bits.iter().any(|&b| b != bits[0]) {
        return Err(Error::unsupported(
            env!("CARGO_PKG_NAME"),
            "TIFF: mixed bit depths not supported",
        ));
    }
    let bps = bits[0];
    if matches!(
        compression,
        Compression::CcittRle | Compression::CcittGroup4Fax
    ) && bps != 1
    {
        return Err(Error::unsupported(
            env!("CARGO_PKG_NAME"),
            "TIFF: CCITT coding requires a bilevel image",
        ));
    }
    let use_predictor = Predictor::try_from(ifd.get_u32(tags::PREDICTOR).unwrap_or(1))?
        == Predictor::HorizontalDifferencing;
    if use_predictor && bps != 8 {
        return Err(Error::unsupported(
            env!("CARGO_PKG_NAME"),
            "TIFF: predictor requires 8-bit samples",
        ));
    }

    let photometric = PhotometricInterpretation::try_from(require_u32(
        ifd,
        tags::PHOTOMETRIC_INTERPRETATION,
        "TIFF: missing PhotometricInterpretation",
    )?)?;
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
            let cm = ifd.get_u32_vec(tags::COLOR_MAP).ok_or_else(|| {
                Error::invalid_input(
                    env!("CARGO_PKG_NAME"),
                    "TIFF: palette image missing ColorMap",
                )
            })?;
            Mode::Palette(Box::new(Palette8::from_tiff_colormap(&cm)?))
        }
        _ => {
            return Err(Error::unsupported(
                env!("CARGO_PKG_NAME"),
                "TIFF: photometric/sample combination not supported yet",
            ));
        }
    };

    // Bytes of one stored (packed) row, before unpacking to 8-bit output samples.
    let stored_row_bytes = match bps {
        8 => width
            .checked_mul(spp)
            .ok_or_else(|| Error::invalid_input(env!("CARGO_PKG_NAME"), "TIFF: image too large"))?,
        1 => width.div_ceil(8), // spp == 1, guaranteed by the match above
        _ => {
            return Err(Error::unsupported(
                env!("CARGO_PKG_NAME"),
                "TIFF: only 1- and 8-bit samples supported so far",
            ));
        }
    };
    let stored_total = stored_row_bytes
        .checked_mul(height)
        .ok_or_else(|| Error::invalid_input(env!("CARGO_PKG_NAME"), "TIFF: image too large"))?;
    if stored_total > MAX_IMAGE_BYTES {
        return Err(Error::unsupported(
            env!("CARGO_PKG_NAME"),
            "TIFF: image exceeds the size limit",
        ));
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
    let tiled = ifd.get(tags::TILE_WIDTH).is_some();
    let mut packed = if tiled {
        decode_tiles(ifd, data, &layout, use_predictor)?
    } else {
        decode_strips(ifd, data, &layout)?
    };
    debug_assert_eq!(packed.len(), stored_total);

    // Reverse the horizontal-differencing predictor (8-bit only) before unpacking.
    if use_predictor && !tiled {
        predictor::reverse(&mut packed, stored_row_bytes, spp);
    }

    // Unpack the stored bytes into 8-bit output samples per the photometric mode.
    let (format, pixels) = match mode {
        Mode::Rgb => (PixelFormat::Rgb8, packed),
        Mode::Rgba => (PixelFormat::Rgba8, packed),
        Mode::Cmyk => (PixelFormat::Cmyk8, packed),
        Mode::Gray { white_is_zero } if bps == 8 => {
            let mut px = packed;
            if white_is_zero {
                for v in &mut px {
                    *v = 255 - *v;
                }
            }
            (PixelFormat::Gray8, px)
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
            // Already expanded to full-range 0/255 samples, so this is Gray8 rather than Bilevel.
            (PixelFormat::Gray8, px)
        }
        Mode::Palette(palette) => {
            // Each 8-bit index selects an RGB triple from the colour table.
            let mut px = Vec::with_capacity(width * height * 3);
            for &idx in &packed {
                px.extend_from_slice(&palette.entry(idx));
            }
            (PixelFormat::Rgb8, px)
        }
    };

    Ok(DecodedImage {
        dims: Dimensions {
            width: width as u32,
            height: height as u32,
        },
        format,
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

/// Decompresses one strip/tile of byte-level data (`None`/PackBits/LZW/Deflate) to `want` bytes.
fn decompress_simple(raw: &[u8], want: usize, compression: Compression) -> Result<Vec<u8>> {
    match compression {
        Compression::None => raw.get(..want).map(<[u8]>::to_vec).ok_or_else(|| {
            Error::invalid_input(env!("CARGO_PKG_NAME"), "TIFF: block shorter than expected")
        }),
        Compression::PackBits => packbits::decode(raw, want),
        Compression::Lzw => lzw::decode(raw, want),
        Compression::Deflate => deflate::decode(raw, want),
        _ => Err(Error::unsupported(
            env!("CARGO_PKG_NAME"),
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
    let offsets = ifd.get_u32_vec(tags::STRIP_OFFSETS).ok_or_else(|| {
        Error::invalid_input(env!("CARGO_PKG_NAME"), "TIFF: missing StripOffsets")
    })?;
    let counts = ifd.get_u32_vec(tags::STRIP_BYTE_COUNTS).ok_or_else(|| {
        Error::invalid_input(env!("CARGO_PKG_NAME"), "TIFF: missing StripByteCounts")
    })?;
    let strips = l.height.div_ceil(rows_per_strip);
    if offsets.len() != strips || counts.len() != strips {
        return Err(Error::invalid_input(
            env!("CARGO_PKG_NAME"),
            "TIFF: strip count mismatch",
        ));
    }
    let mut packed = Vec::with_capacity(l.stored_row_bytes * l.height);
    for (i, (&off, &cnt)) in offsets.iter().zip(&counts).enumerate() {
        let rows = rows_per_strip.min(l.height - i * rows_per_strip);
        let want = rows * l.stored_row_bytes;
        let raw = data
            .get(off as usize..off as usize + cnt as usize)
            .ok_or_else(|| {
                Error::invalid_input(env!("CARGO_PKG_NAME"), "TIFF: strip out of bounds")
            })?;
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
fn decode_tiles(ifd: &Ifd, data: &[u8], l: &Layout, use_predictor: bool) -> Result<Vec<u8>> {
    if l.bps != 8 {
        return Err(Error::unsupported(
            env!("CARGO_PKG_NAME"),
            "TIFF: tiled images supported only for 8-bit samples so far",
        ));
    }
    let tw = ifd
        .get_u32(tags::TILE_WIDTH)
        .ok_or_else(|| Error::invalid_input(env!("CARGO_PKG_NAME"), "TIFF: missing TileWidth"))?
        as usize;
    let th = ifd
        .get_u32(tags::TILE_LENGTH)
        .ok_or_else(|| Error::invalid_input(env!("CARGO_PKG_NAME"), "TIFF: missing TileLength"))?
        as usize;
    if tw == 0 || th == 0 {
        return Err(Error::invalid_input(
            env!("CARGO_PKG_NAME"),
            "TIFF: zero tile dimension",
        ));
    }
    let offsets = ifd
        .get_u32_vec(tags::TILE_OFFSETS)
        .ok_or_else(|| Error::invalid_input(env!("CARGO_PKG_NAME"), "TIFF: missing TileOffsets"))?;
    let counts = ifd.get_u32_vec(tags::TILE_BYTE_COUNTS).ok_or_else(|| {
        Error::invalid_input(env!("CARGO_PKG_NAME"), "TIFF: missing TileByteCounts")
    })?;
    let across = l.width.div_ceil(tw);
    let down = l.height.div_ceil(th);
    if offsets.len() != across * down || counts.len() != across * down {
        return Err(Error::invalid_input(
            env!("CARGO_PKG_NAME"),
            "TIFF: tile count mismatch",
        ));
    }
    let tile_row_bytes = tw
        .checked_mul(l.spp)
        .ok_or_else(|| Error::invalid_input(env!("CARGO_PKG_NAME"), "TIFF: tile too large"))?;
    let tile_size = th
        .checked_mul(tile_row_bytes)
        .ok_or_else(|| Error::invalid_input(env!("CARGO_PKG_NAME"), "TIFF: tile too large"))?;
    if tile_size > MAX_IMAGE_BYTES {
        return Err(Error::unsupported(
            env!("CARGO_PKG_NAME"),
            "TIFF: tile exceeds the size limit",
        ));
    }
    let mut packed = vec![0u8; l.stored_row_bytes * l.height];
    for ty in 0..down {
        for tx in 0..across {
            let idx = ty * across + tx;
            let (off, cnt) = (offsets[idx] as usize, counts[idx] as usize);
            let raw = data.get(off..off + cnt).ok_or_else(|| {
                Error::invalid_input(env!("CARGO_PKG_NAME"), "TIFF: tile out of bounds")
            })?;
            let mut tile = decompress_simple(raw, tile_size, l.compression)?;
            if use_predictor {
                predictor::reverse(&mut tile, tile_row_bytes, l.spp);
            }
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
    use gamut_core::{EncodeImage, ImageRef};
    use gamut_ifd::ByteOrder;

    use super::*;
    use crate::encoder::TiffEncoder;

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
