//! Input decoding: turn an image file into interleaved 8-bit RGB for the gamut encoders.
//!
//! Decoding borrows the third-party [`image`] crate (PNG/JPEG/PPM). Everything downstream —
//! the actual encode — stays in the gamut crates, so the memory-safe encode path is preserved.

use std::path::Path;

use gamut::core::Dimensions;

use crate::error::CliError;

/// Decodes a supported image file (PNG, JPEG, or PPM/P6) into interleaved 8-bit RGB.
///
/// Returns the pixel buffer (`width * height * 3` bytes, row-major, no padding) and its
/// dimensions. Alpha is dropped and grayscale is expanded so the buffer is always 3 bytes per
/// pixel, matching the gamut encoders' input contract. The format is detected from the file
/// contents, so the extension need not be accurate.
pub(crate) fn decode_rgb8(path: &Path) -> Result<(Vec<u8>, Dimensions), CliError> {
    let reader = image::ImageReader::open(path).map_err(|source| CliError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let decoded = reader.decode().map_err(|source| CliError::Decode {
        path: path.to_path_buf(),
        source,
    })?;
    let rgb = decoded.to_rgb8();
    let (width, height) = rgb.dimensions();
    Ok((rgb.into_raw(), Dimensions { width, height }))
}
