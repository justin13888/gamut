//! Input decoding: turn an image file into interleaved 8-bit RGB(A) for the gamut encoders.
//!
//! PNG/JPEG/PPM are decoded with the third-party [`image`] crate; **WebP** and **JPEG XL** are
//! decoded by gamut's own decoders ([`gamut::webp::WebpDecoder`] and [`gamut::jxl::JxlDecoder`], the
//! latter the pure-Rust jxl-rs backend), so those paths need no third-party image library.
//! Everything downstream — the actual encode — is produced by the gamut crates regardless of input
//! format.

use std::path::Path;

use gamut::core::{DecodeImage, Dimensions, ImageBuf, Rgb8, Rgba8};
use gamut::jxl::JxlDecoder;
use gamut::webp::WebpDecoder;

use crate::error::CliError;

/// Decodes a supported image file (PNG, JPEG, PPM/P6, WebP, or JPEG XL) into interleaved 8-bit RGB.
///
/// Returns the pixel buffer (`width * height * 3` bytes, row-major, no padding) and its
/// dimensions. Alpha is dropped and grayscale is expanded so the buffer is always 3 bytes per
/// pixel, matching the gamut encoders' input contract. The format is detected from the file
/// contents, so the extension need not be accurate.
pub(crate) fn decode_rgb8(path: &Path) -> Result<(Vec<u8>, Dimensions), CliError> {
    decode(path, false)
}

/// Decodes a supported image file (PNG, JPEG, PPM/P6, WebP, or JPEG XL) into interleaved 8-bit RGBA,
/// keeping the alpha channel (fully opaque when the source has none). Returns `width * height * 4`
/// bytes, row-major. The format is detected from the file contents.
pub(crate) fn decode_rgba8(path: &Path) -> Result<(Vec<u8>, Dimensions), CliError> {
    decode(path, true)
}

/// Reads `path` and decodes it to interleaved RGB (`want_alpha == false`) or RGBA
/// (`want_alpha == true`).
fn decode(path: &Path, want_alpha: bool) -> Result<(Vec<u8>, Dimensions), CliError> {
    let bytes = std::fs::read(path).map_err(|source| CliError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    decode_bytes(path, &bytes, want_alpha)
}

/// Format-dispatching core: routes WebP and JPEG XL streams to gamut's own decoders and everything
/// else to the `image` crate (which has no JPEG XL support). Split out from [`decode`] so it is
/// unit-testable without touching the filesystem; `path` is used only to label errors.
fn decode_bytes(
    path: &Path,
    bytes: &[u8],
    want_alpha: bool,
) -> Result<(Vec<u8>, Dimensions), CliError> {
    if is_webp(bytes) {
        let decoder = WebpDecoder::new();
        // `?` maps `gamut::core::Error` to `CliError::Codec` via the existing `#[from]` impl.
        return Ok(if want_alpha {
            let img: ImageBuf<Rgba8> = decoder.decode_image(bytes)?;
            let dims = img.dimensions();
            (img.into_samples(), dims)
        } else {
            let img: ImageBuf<Rgb8> = decoder.decode_image(bytes)?;
            let dims = img.dimensions();
            (img.into_samples(), dims)
        });
    }

    if is_jxl(bytes) {
        let decoder = JxlDecoder::new();
        // `?` maps `gamut::core::Error` to `CliError::Codec` via the existing `#[from]` impl.
        return Ok(if want_alpha {
            let img: ImageBuf<Rgba8> = decoder.decode_image(bytes)?;
            let dims = img.dimensions();
            (img.into_samples(), dims)
        } else {
            let img: ImageBuf<Rgb8> = decoder.decode_image(bytes)?;
            let dims = img.dimensions();
            (img.into_samples(), dims)
        });
    }

    let decoded = image::load_from_memory(bytes).map_err(|source| CliError::Decode {
        path: path.to_path_buf(),
        source,
    })?;
    let (buf, width, height) = if want_alpha {
        let rgba = decoded.to_rgba8();
        let (w, h) = rgba.dimensions();
        (rgba.into_raw(), w, h)
    } else {
        let rgb = decoded.to_rgb8();
        let (w, h) = rgb.dimensions();
        (rgb.into_raw(), w, h)
    };
    Ok((buf, Dimensions { width, height }))
}

/// Returns `true` if `bytes` begins with a RIFF/WebP container signature (`RIFF`…`WEBP`), matching
/// how the `image` crate detects formats by content rather than trusting the file extension.
fn is_webp(bytes: &[u8]) -> bool {
    bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP"
}

/// Returns `true` if `bytes` begins with a JPEG XL signature — either the 2-byte bare codestream
/// signature `FF 0A` or the 12-byte ISO BMFF container box signature
/// (`00 00 00 0C 4A 58 4C 20 0D 0A 87 0A`). Sniffs by content like [`is_webp`], so the file
/// extension need not be accurate.
fn is_jxl(bytes: &[u8]) -> bool {
    /// The ISO BMFF `.jxl` container signature: a 12-byte JXL box.
    const CONTAINER: [u8; 12] = [
        0x00, 0x00, 0x00, 0x0C, 0x4A, 0x58, 0x4C, 0x20, 0x0D, 0x0A, 0x87, 0x0A,
    ];
    bytes.starts_with(&[0xFF, 0x0A]) || bytes.starts_with(&CONTAINER)
}

#[cfg(test)]
mod tests {
    use gamut::core::{EncodeImage, ImageRef};
    use gamut::jxl::JxlEncoder;
    use gamut::webp::WebpEncoder;

    use super::*;

    /// Encodes `rgba` as a lossless (so bit-exact) WebP file for the round-trip tests.
    fn lossless_webp(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        WebpEncoder::lossless()
            .encode_image(
                ImageRef::<Rgba8>::new(rgba, Dimensions { width, height }).unwrap(),
                &mut out,
            )
            .expect("encode webp");
        out
    }

    /// Encodes `rgba` as a lossless (so bit-exact) JPEG XL codestream for the round-trip tests.
    fn lossless_jxl(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        JxlEncoder::lossless()
            .encode_image(
                ImageRef::<Rgba8>::new(rgba, Dimensions { width, height }).unwrap(),
                &mut out,
            )
            .expect("encode jxl");
        out
    }

    #[test]
    fn sniffs_and_decodes_opaque_webp_to_rgb8() {
        let rgba = [
            0x10, 0x20, 0x30, 0xff, // px (0,0)
            0x40, 0x50, 0x60, 0xff, // px (1,0)
            0x70, 0x80, 0x90, 0xff, // px (0,1)
            0xa0, 0xb0, 0xc0, 0xff, // px (1,1)
        ];
        let webp = lossless_webp(2, 2, &rgba);
        assert!(is_webp(&webp));
        let (rgb, dims) = decode_bytes(Path::new("mem.webp"), &webp, false).unwrap();
        assert_eq!(
            dims,
            Dimensions {
                width: 2,
                height: 2
            }
        );
        // Lossless: RGB survives exactly; alpha is dropped.
        assert_eq!(
            rgb,
            [
                0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x90, 0xa0, 0xb0, 0xc0
            ]
        );
    }

    #[test]
    fn decodes_transparent_webp_to_rgba8() {
        // One pixel per row; alpha ranges from fully transparent to fully opaque.
        #[rustfmt::skip]
        let rgba = [
            0x11, 0x22, 0x33, 0x00,
            0x44, 0x55, 0x66, 0x80,
            0x77, 0x88, 0x99, 0xc0,
            0xaa, 0xbb, 0xcc, 0xff,
        ];
        let webp = lossless_webp(2, 2, &rgba);
        let (out, dims) = decode_bytes(Path::new("mem.webp"), &webp, true).unwrap();
        assert_eq!(
            dims,
            Dimensions {
                width: 2,
                height: 2
            }
        );
        // Lossless VP8L carries alpha natively, so the round-trip is bit-exact.
        assert_eq!(out, rgba);
    }

    #[test]
    fn sniffs_and_decodes_opaque_jxl_to_rgb8() {
        let rgba = [
            0x10, 0x20, 0x30, 0xff, // px (0,0)
            0x40, 0x50, 0x60, 0xff, // px (1,0)
            0x70, 0x80, 0x90, 0xff, // px (0,1)
            0xa0, 0xb0, 0xc0, 0xff, // px (1,1)
        ];
        let jxl = lossless_jxl(2, 2, &rgba);
        // A bare codestream starts with the 2-byte JXL signature.
        assert_eq!(&jxl[0..2], &[0xFF, 0x0A]);
        assert!(is_jxl(&jxl));
        let (rgb, dims) = decode_bytes(Path::new("mem.jxl"), &jxl, false).unwrap();
        assert_eq!(
            dims,
            Dimensions {
                width: 2,
                height: 2
            }
        );
        // Lossless: RGB survives exactly; alpha is dropped.
        assert_eq!(
            rgb,
            [
                0x10, 0x20, 0x30, 0x40, 0x50, 0x60, 0x70, 0x80, 0x90, 0xa0, 0xb0, 0xc0
            ]
        );
    }

    #[test]
    fn decodes_transparent_jxl_to_rgba8() {
        // One pixel per row; alpha ranges from fully transparent to fully opaque.
        #[rustfmt::skip]
        let rgba = [
            0x11, 0x22, 0x33, 0x00,
            0x44, 0x55, 0x66, 0x80,
            0x77, 0x88, 0x99, 0xc0,
            0xaa, 0xbb, 0xcc, 0xff,
        ];
        let jxl = lossless_jxl(2, 2, &rgba);
        assert!(is_jxl(&jxl));
        let (out, dims) = decode_bytes(Path::new("mem.jxl"), &jxl, true).unwrap();
        assert_eq!(
            dims,
            Dimensions {
                width: 2,
                height: 2
            }
        );
        // Lossless JPEG XL carries alpha natively, so the round-trip is bit-exact.
        assert_eq!(out, rgba);
    }

    #[test]
    fn jxl_container_signature_is_sniffed() {
        // The 12-byte ISO BMFF `.jxl` box signature is recognised as JPEG XL.
        let container = [
            0x00, 0x00, 0x00, 0x0C, 0x4A, 0x58, 0x4C, 0x20, 0x0D, 0x0A, 0x87, 0x0A,
        ];
        assert!(is_jxl(&container));
        // The bare-codestream signature is also recognised; unrelated bytes are not.
        assert!(is_jxl(&[0xFF, 0x0A, 0x00]));
        assert!(!is_jxl(&[0xFF, 0x0B]));
        assert!(!is_jxl(b"RIFF"));
    }

    #[test]
    fn non_riff_bytes_take_the_image_path() {
        // Not a RIFF/WebP container, so it must NOT be routed to the webp decoder; the image path
        // rejects it with a decode error rather than panicking.
        let junk = [0u8; 32];
        assert!(!is_webp(&junk));
        let err = decode_bytes(Path::new("junk.bin"), &junk, false).unwrap_err();
        assert!(matches!(err, CliError::Decode { .. }));
    }
}
