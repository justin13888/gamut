//! What a TIFF page declares about its stored pixels, read from tags alone.
//!
//! This module *describes*; it does not judge. Which depths, sample formats and photometric
//! interpretations this crate can actually decode is policy, and policy lives in the decoder — so a
//! page `gamut-tiff` cannot decode can still be inspected here. Keeping the two apart is what lets
//! a caller dispatch on a page's declared layout before committing to a decode, and it keeps the
//! defaults for absent tags in one place instead of drifting between the probe and the decoder.

use gamut_core::{Error, Result};
use gamut_ifd::{ByteOrder, Ifd};

use crate::compression::Compression;
use crate::ifd::{PhotometricInterpretation, Predictor};
use crate::tags;

/// Reads a required unsigned-integer tag.
fn require_u32(ifd: &Ifd, tag: u16, what: &'static str) -> Result<u32> {
    ifd.get_u32(tag)
        .ok_or_else(|| Error::invalid_input(env!("CARGO_PKG_NAME"), what))
}

/// What a TIFF page declares about its stored pixels.
///
/// Every field is reported **as declared**, with the TIFF 6.0 defaults applied for absent tags
/// (`SamplesPerPixel = 1`, `BitsPerSample = 1`, `Compression = None`, `Predictor = 1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TiffInfo {
    /// Image width in pixels (`ImageWidth`, 256).
    pub(crate) width: u32,
    /// Image height in pixels (`ImageLength`, 257).
    pub(crate) height: u32,
    /// Bits per sample (`BitsPerSample`, 258). Every sample shares one depth.
    pub(crate) bits_per_sample: u32,
    /// How samples map to colour (`PhotometricInterpretation`, 262).
    pub(crate) photometric: PhotometricInterpretation,
    /// Components per pixel (`SamplesPerPixel`, 277).
    pub(crate) samples_per_pixel: u32,
    /// The compression scheme (`Compression`, 259).
    pub(crate) compression: Compression,
    /// The prediction scheme applied before compression (`Predictor`, 317).
    pub(crate) predictor: Predictor,
    /// Whether the page stores tiles (`TileWidth` present) rather than strips.
    pub(crate) tiled: bool,
    /// The byte order of the file the page belongs to (`II` or `MM`).
    pub(crate) byte_order: ByteOrder,
}

/// Reads one page's pixel-layout tags, applying TIFF's defaults for the absent ones.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if a required tag is missing or the page is zero-sized, or
/// [`Error::Unsupported`] for an on-disk code this crate does not recognise or a layout a single
/// depth cannot describe.
pub(crate) fn page_info(ifd: &Ifd, byte_order: ByteOrder) -> Result<TiffInfo> {
    let width = require_u32(ifd, tags::IMAGE_WIDTH, "TIFF: missing ImageWidth")?;
    let height = require_u32(ifd, tags::IMAGE_LENGTH, "TIFF: missing ImageLength")?;
    if width == 0 || height == 0 {
        return Err(Error::invalid_input(
            env!("CARGO_PKG_NAME"),
            "TIFF: zero-sized image",
        ));
    }

    let compression = Compression::try_from(ifd.get_u32(tags::COMPRESSION).unwrap_or(1))?;
    let samples_per_pixel = ifd.get_u32(tags::SAMPLES_PER_PIXEL).unwrap_or(1);

    // BitsPerSample (258) is one value per sample. This crate's sample model gives every component
    // the same depth, so a page whose samples disagree cannot be described — let alone decoded.
    let bits = ifd
        .get_u32_vec(tags::BITS_PER_SAMPLE)
        .unwrap_or_else(|| vec![1; samples_per_pixel as usize]);
    if bits.len() != samples_per_pixel as usize || bits.iter().any(|&b| b != bits[0]) {
        return Err(Error::unsupported(
            env!("CARGO_PKG_NAME"),
            "TIFF: mixed bit depths not supported",
        ));
    }

    let photometric = PhotometricInterpretation::try_from(require_u32(
        ifd,
        tags::PHOTOMETRIC_INTERPRETATION,
        "TIFF: missing PhotometricInterpretation",
    )?)?;
    let predictor = Predictor::try_from(ifd.get_u32(tags::PREDICTOR).unwrap_or(1))?;

    Ok(TiffInfo {
        width,
        height,
        bits_per_sample: bits[0],
        photometric,
        samples_per_pixel,
        compression,
        predictor,
        tiled: ifd.get(tags::TILE_WIDTH).is_some(),
        byte_order,
    })
}
