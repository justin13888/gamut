//! The TIFF decoder.

/// Decoder for TIFF 6.0 images.
///
/// Will implement [`gamut_core::Decoder`], reading the byte-order header, following IFD/value
/// offsets, and decoding the strip/tile image data of the first subfile (or any page).
/// Implementation pending (see issue #107).
pub struct TiffDecoder;
