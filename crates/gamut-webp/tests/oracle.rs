//! Differential oracle (libwebp) for the VP8L lossless decoder.
//!
//! libwebp is the third-party reference: its lossless encoder freely uses predictors, the color
//! transform, LZ77 backward references, and the color cache, so decoding its output through gamut
//! exercises the whole decode surface. The checks here are:
//!   - libwebp self-round-trip (proves the FFI harness + linked libwebp build);
//!   - **libwebp encode → gamut decode == source** (the native decoder reproduces libwebp's output).
//!
//! The encoder-side differential (gamut encode → libwebp decode == source) lands with the encoder
//! commits.

mod common;

use common::{libwebp_decode_rgba, libwebp_encode_lossless_rgba, libwebp_get_info, pattern_rgba};
use gamut_webp::WebpDecoder;

/// The standard dimension matrix exercised by the differential tests, including the awkward
/// single-row / single-column / non-power-of-two cases.
const DIMENSIONS: &[(u32, u32)] = &[
    (1, 1),
    (2, 2),
    (16, 16),
    (17, 9),
    (64, 48),
    (255, 1),
    (1, 255),
];

/// Drops the alpha byte of an interleaved RGBA buffer, yielding interleaved RGB.
fn rgba_to_rgb(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect()
}

#[test]
fn libwebp_lossless_self_roundtrip() {
    // Encode → get_info → decode entirely within libwebp: the wrappers and the linked libwebp build
    // are correct iff a fully-opaque image survives a lossless round-trip bit-exactly.
    for (w, h) in [(1u32, 1u32), (16, 16), (17, 9), (64, 48)] {
        let rgba = pattern_rgba(w, h);
        let webp = libwebp_encode_lossless_rgba(&rgba, w, h);
        assert!(!webp.is_empty(), "encode produced no bytes at {w}x{h}");
        assert_eq!(
            libwebp_get_info(&webp),
            Some((w, h)),
            "get_info mismatch at {w}x{h}"
        );
        let decoded = libwebp_decode_rgba(&webp);
        assert_eq!((decoded.width, decoded.height), (w, h));
        assert_eq!(
            decoded.rgba, rgba,
            "lossless must round-trip bit-exactly at {w}x{h}"
        );
    }
}

#[test]
fn gamut_decodes_libwebp_lossless_to_source() {
    // libwebp encodes (using whatever transforms/LZ77/cache it likes); gamut must decode back to the
    // exact source pixels. This is the lossless guarantee end to end through the native decoder.
    for &(w, h) in DIMENSIONS {
        let rgba = pattern_rgba(w, h);
        let webp = libwebp_encode_lossless_rgba(&rgba, w, h);
        let mut out = Vec::new();
        let dims = WebpDecoder::new()
            .decode_to_rgb8(&webp, &mut out)
            .expect("gamut decode");
        assert_eq!(
            (dims.width, dims.height),
            (w, h),
            "dims mismatch at {w}x{h}"
        );
        assert_eq!(out, rgba_to_rgb(&rgba), "pixel mismatch at {w}x{h}");
    }
}
