//! Bounded zlib inflation (RFC 1950/1951) — the decoder's one point of contact with
//! [`miniz_oxide`].
//!
//! PNG compressed payloads (IDAT, iCCP, zTXt, compressed iTXt) are attacker-controlled, so every
//! inflation carries a hard output cap: a tiny stream claiming to inflate without bound (a "zlib
//! bomb") fails cleanly instead of exhausting memory.

use gamut_core::{Error, Result};
use miniz_oxide::inflate::TINFLStatus;

/// Inflates a zlib stream, refusing to produce more than `max_out` bytes.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if the stream is corrupt, truncated, or would inflate past
/// `max_out`.
pub(crate) fn inflate_zlib(data: &[u8], max_out: usize) -> Result<Vec<u8>> {
    miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(data, max_out).map_err(|e| {
        match e.status {
            TINFLStatus::HasMoreOutput => {
                Error::InvalidInput("PNG: compressed data inflates past the expected size")
            }
            _ => Error::InvalidInput("PNG: corrupt zlib stream"),
        }
    })
}

#[cfg(test)]
mod tests {
    use gamut_deflate::DeflateEncoder;

    use super::*;

    #[test]
    fn round_trips_gamut_deflate_output() {
        let payload: Vec<u8> = (0..2000u32).map(|i| (i * 7) as u8).collect();
        let mut zlib = Vec::new();
        DeflateEncoder::new().zlib_compress(&payload, &mut zlib);
        assert_eq!(inflate_zlib(&zlib, payload.len()).unwrap(), payload);
    }

    #[test]
    fn caps_output_size() {
        let payload = vec![0u8; 4096];
        let mut zlib = Vec::new();
        DeflateEncoder::new().zlib_compress(&payload, &mut zlib);
        // The stream is valid but inflates past the cap: that must be the *cap* error — the
        // distinction from plain corruption matters because a decoder reports a bomb, not a
        // damaged file.
        match inflate_zlib(&zlib, payload.len() - 1) {
            Err(Error::InvalidInput(msg)) => {
                assert!(
                    msg.contains("inflates past"),
                    "cap-specific message, got {msg:?}"
                );
            }
            other => panic!("expected the cap error, got {other:?}"),
        }
        assert!(inflate_zlib(&zlib, payload.len()).is_ok());
    }

    #[test]
    fn rejects_garbage_and_truncation() {
        assert!(inflate_zlib(&[], 16).is_err());
        assert!(inflate_zlib(&[0xFF, 0xFF, 0x00], 16).is_err()); // bad zlib header
        let mut zlib = Vec::new();
        DeflateEncoder::new().zlib_compress(&[1, 2, 3, 4], &mut zlib);
        assert!(inflate_zlib(&zlib[..zlib.len() - 1], 16).is_err()); // truncated Adler-32
        let last = zlib.len() - 1;
        zlib[last] ^= 0xFF;
        assert!(inflate_zlib(&zlib, 16).is_err()); // wrong Adler-32
    }
}
