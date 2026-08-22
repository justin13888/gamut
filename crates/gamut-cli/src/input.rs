//! Input decoding: turn an image file into interleaved 8-bit RGB(A) for the gamut encoders.
//!
//! **PNG, JPEG, WebP, and JPEG XL are decoded by gamut's own decoders.** Only PPM still goes
//! through the third-party [`image`] crate, because gamut has no PPM decoder. Everything
//! downstream — the actual encode — is produced by the gamut crates regardless of input format.
//!
//! Asking every decoder for a fixed `Rgb8`/`Rgba8` buffer is a *lossy* request: a 16-bit PNG has to
//! narrow, a grayscale JPEG has to replicate, a transparent WebP asked for RGB has to drop its
//! alpha. gamut's decoders refuse that by default, which is the right default for a library and the
//! wrong one for this CLI — so every decoder here is given
//! [`ConvertPolicy::permissive`](gamut::core::convert::ConvertPolicy::permissive), the CLI opting
//! into the loss explicitly on the user's behalf.

use std::path::Path;

use gamut::core::convert::{ConvertPolicy, convert};
use gamut::core::{DecodeImage, Dimensions, ImageBuf, Rgb8, Rgba8};
use gamut::jpeg::JpegDecoder;
use gamut::jxl::JxlDecoder;
use gamut::png::PngDecoder;
use gamut::webp::WebpDecoder;

use crate::error::CliError;

/// Unwraps a gamut decode into the flat `(samples, dimensions)` pair the encoders take, choosing
/// the RGB or RGBA layout by `want_alpha`.
///
/// A macro rather than a function because the two arms instantiate `DecodeImage` at different
/// pixel types, which no single call can express.
macro_rules! decode_with {
    ($decoder:expr, $bytes:expr, $want_alpha:expr) => {{
        let decoder = $decoder;
        // `?` maps `gamut::core::Error` to `CliError::Codec` via the existing `#[from]` impl.
        if $want_alpha {
            let img: ImageBuf<Rgba8> = decoder.decode_image($bytes)?;
            let dims = img.dimensions();
            (img.into_samples(), dims)
        } else {
            let img: ImageBuf<Rgb8> = decoder.decode_image($bytes)?;
            let dims = img.dimensions();
            (img.into_samples(), dims)
        }
    }};
}

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

/// Format-dispatching core: routes every format gamut can decode to gamut's own decoder, and the
/// remainder (PPM) to the `image` crate. Split out from [`decode`] so it is unit-testable without
/// touching the filesystem; `path` is used only to label errors.
fn decode_bytes(
    path: &Path,
    bytes: &[u8],
    want_alpha: bool,
) -> Result<(Vec<u8>, Dimensions), CliError> {
    let lossy = ConvertPolicy::permissive();

    if is_webp(bytes) {
        return Ok(decode_with!(
            WebpDecoder::new().convert_policy(lossy),
            bytes,
            want_alpha
        ));
    }

    if is_jxl(bytes) {
        return Ok(decode_with!(
            JxlDecoder::new().with_convert_policy(lossy),
            bytes,
            want_alpha
        ));
    }

    if is_png(bytes) {
        return Ok(decode_with!(
            PngDecoder::new().convert_policy(lossy),
            bytes,
            want_alpha
        ));
    }

    if is_jpeg(bytes) {
        // A JPEG never carries alpha, so gamut-jpeg implements no `DecodeImage<Rgba8>`. Decode the
        // colour it does carry, then let the same conversion engine add the opaque alpha channel
        // rather than padding it by hand here.
        let img: ImageBuf<Rgb8> = JpegDecoder::new()
            .convert_policy(lossy)
            .decode_image(bytes)?;
        if !want_alpha {
            let dims = img.dimensions();
            return Ok((img.into_samples(), dims));
        }
        let rgba: ImageBuf<Rgba8> = convert(img.as_ref(), lossy)?;
        let dims = rgba.dimensions();
        return Ok((rgba.into_samples(), dims));
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

/// Returns `true` if `bytes` begins with the 8-byte PNG signature (§5.2). Sniffs by content like
/// [`is_webp`], so the file extension need not be accurate.
fn is_png(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A])
}

/// Returns `true` if `bytes` begins with the JPEG SOI marker (`FF D8 FF`). Sniffs by content like
/// [`is_webp`], so the file extension need not be accurate.
fn is_jpeg(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0xFF, 0xD8, 0xFF])
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
    fn sixteen_bit_grayscale_png_is_decoded_by_gamut() {
        use gamut::core::Gray16;
        use gamut::png::PngEncoder;

        // Two things gamut-png refuses by default happen here at once: 16-bit samples narrow to 8,
        // and a single grey channel replicates into three. The decode therefore succeeds only
        // because this module opts into the loss -- and only because the PNG path now goes through
        // gamut-png at all instead of detouring through the `image` crate.
        let dims = Dimensions {
            width: 4,
            height: 1,
        };
        let gray16: [u16; 4] = [0, 0x8080, 0xFFFF, 0x0101];
        let mut png = Vec::new();
        PngEncoder::new()
            .encode_image(ImageRef::<Gray16>::new(&gray16, dims).unwrap(), &mut png)
            .expect("encode png");
        assert!(is_png(&png));

        let (rgb, got) = decode_bytes(Path::new("mem.png"), &png, false).unwrap();
        assert_eq!(got, dims);
        // 0x8080 -> 128 and 0x0101 -> 1 are the round-to-nearest narrowing; a truncating `>> 8`
        // would give 255 -> 255 but 0x8080 -> 128 and 0xFFFF -> 255 only by luck, so the endpoints
        // plus 0x0101 pin the scale.
        assert_eq!(rgb, [0, 0, 0, 128, 128, 128, 255, 255, 255, 1, 1, 1]);
    }

    #[test]
    fn grayscale_jpeg_is_decoded_by_gamut_and_gains_opaque_alpha() {
        use gamut::core::Gray8;
        use gamut::jpeg::JpegEncoder;

        // gamut-jpeg has no `DecodeImage<Rgba8>` (a JPEG carries no alpha), so this exercises the
        // decode-then-widen path: grey replicates into RGB and the shared engine appends opaque
        // alpha.
        let dims = Dimensions {
            width: 8,
            height: 8,
        };
        let gray = vec![0x40u8; 64];
        let mut jpeg = Vec::new();
        JpegEncoder::new()
            .encode_image(ImageRef::<Gray8>::new(&gray, dims).unwrap(), &mut jpeg)
            .expect("encode jpeg");
        assert!(is_jpeg(&jpeg));

        let (rgba, got) = decode_bytes(Path::new("mem.jpg"), &jpeg, true).unwrap();
        assert_eq!(got, dims);
        assert_eq!(rgba.len(), 64 * 4);
        for px in rgba.as_chunks::<4>().0 {
            // Lossy JPEG, so the grey level is approximate -- but it must stay grey (R == G == B)
            // and every pixel must be fully opaque.
            assert_eq!(px[0], px[1]);
            assert_eq!(px[1], px[2]);
            assert_eq!(px[3], 0xff);
        }
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
