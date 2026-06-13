//! Compressing and decompressing the packed DNG sample stream.
//!
//! The codec wraps the [`crate::bitpack`] byte stream: the encoder packs samples then compresses
//! each strip; the decoder decompresses each strip then unpacks. DNG's **Deflate** (`Compression =
//! 8`) is zlib-format (RFC 1950), matching what the reference implementation reads/writes via
//! zlib's `compress2`/`uncompress`. Lossless JPEG and JPEG XL are added in later phases.

use gamut_core::{Error, Result};

use crate::values::Compression;

/// Compresses one already-packed strip with `scheme`.
///
/// # Errors
///
/// Returns [`Error::Unsupported`] for a scheme gamut-dng cannot yet encode.
pub(crate) fn compress(scheme: Compression, packed: &[u8]) -> Result<Vec<u8>> {
    match scheme {
        Compression::Uncompressed => Ok(packed.to_vec()),
        // Level 6 is zlib's default speed/ratio trade-off.
        Compression::Deflate => Ok(miniz_oxide::deflate::compress_to_vec_zlib(packed, 6)),
        _ => Err(Error::Unsupported(
            "DNG: this compression is not yet encodable",
        )),
    }
}

/// Decompresses one strip produced with `scheme` back to the packed sample bytes.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if the compressed data is malformed, or [`Error::Unsupported`]
/// for a scheme gamut-dng cannot yet decode.
pub(crate) fn decompress(scheme: Compression, bytes: &[u8]) -> Result<Vec<u8>> {
    match scheme {
        Compression::Uncompressed => Ok(bytes.to_vec()),
        Compression::Deflate => miniz_oxide::inflate::decompress_to_vec_zlib(bytes)
            .map_err(|_| Error::InvalidInput("DNG: corrupt Deflate stream")),
        _ => Err(Error::Unsupported(
            "DNG: this compression is not yet decodable",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deflate_roundtrips() {
        let data: Vec<u8> = (0..4096).map(|i| (i * 7 % 251) as u8).collect();
        let packed = compress(Compression::Deflate, &data).unwrap();
        assert!(packed.len() < data.len(), "structured data should shrink");
        assert_eq!(decompress(Compression::Deflate, &packed).unwrap(), data);
    }

    #[test]
    fn uncompressed_is_passthrough() {
        assert_eq!(
            compress(Compression::Uncompressed, &[1, 2, 3]).unwrap(),
            vec![1, 2, 3]
        );
        assert_eq!(
            decompress(Compression::Uncompressed, &[1, 2, 3]).unwrap(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn lossless_jpeg_not_yet_encodable() {
        assert!(compress(Compression::LosslessJpeg, &[0; 16]).is_err());
    }
}
