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
use gamut_webp::{Dimensions, WebpDecoder, WebpEncoder};

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

#[test]
fn libwebp_decodes_gamut_lossless_to_source() {
    // The reverse direction: gamut encodes, libwebp (the reference) decodes, and must recover the
    // source pixels — proving gamut emits a conformant lossless stream.
    for &(w, h) in DIMENSIONS {
        let rgb = rgba_to_rgb(&pattern_rgba(w, h));
        let mut webp = Vec::new();
        WebpEncoder::lossless()
            .encode_rgb8(
                &rgb,
                Dimensions {
                    width: w,
                    height: h,
                },
                &mut webp,
            )
            .expect("gamut encode");
        assert_eq!(libwebp_get_info(&webp), Some((w, h)), "get_info at {w}x{h}");
        let decoded = libwebp_decode_rgba(&webp);
        assert_eq!((decoded.width, decoded.height), (w, h));
        assert_eq!(rgba_to_rgb(&decoded.rgba), rgb, "pixel mismatch at {w}x{h}");
    }
}

/// Encodes interleaved RGB with gamut and decodes it with libwebp, asserting the pixels survive.
fn assert_gamut_encode_libwebp_decode(rgb: &[u8], w: u32, h: u32, label: &str) {
    let mut webp = Vec::new();
    WebpEncoder::lossless()
        .encode_rgb8(
            rgb,
            Dimensions {
                width: w,
                height: h,
            },
            &mut webp,
        )
        .expect("gamut encode");
    let decoded = libwebp_decode_rgba(&webp);
    assert_eq!((decoded.width, decoded.height), (w, h), "dims for {label}");
    assert_eq!(rgba_to_rgb(&decoded.rgba), rgb, "pixels for {label}");
}

#[test]
fn libwebp_decodes_every_gamut_encoder_path() {
    // Each image steers gamut's encoder down a different path; libwebp must decode them all.
    let (w, h) = (40u32, 40u32);
    let n = (w * h) as usize;

    // Solid color → palette transform with 8-pixel bundling.
    let solid: Vec<u8> = [30u8, 60, 90].repeat(n);
    assert_gamut_encode_libwebp_decode(&solid, w, h, "solid");

    // Few colors, repetitive → palette + color cache + LZ77.
    let palette = [[10u8, 20, 30], [40, 50, 60], [70, 80, 90]];
    let few: Vec<u8> = (0..n).flat_map(|i| palette[i % 3]).collect();
    assert_gamut_encode_libwebp_decode(&few, w, h, "few-color");

    // 32 colors split top/bottom → palette + multi-group entropy image.
    let regioned: Vec<u8> = (0..n)
        .flat_map(|i| {
            let (x, y) = (i as u32 % w, i as u32 / w);
            let scatter = ((x * 7 + y * 11) % 16) as u8;
            let base = if y < h / 2 { 0 } else { 16 };
            let idx = base + scatter;
            [idx, idx.wrapping_mul(7), idx.wrapping_mul(13)]
        })
        .collect();
    assert_gamut_encode_libwebp_decode(&regioned, w, h, "multi-region");

    // Many colors → spatial transforms (subtract-green/predictor/color) + LZ77 + cache.
    let many: Vec<u8> = (0..n)
        .flat_map(|i| {
            let (x, y) = (i as u32 % w, i as u32 / w);
            [
                (x * 9 + y * 5) as u8,
                (x * 13 + y * 7) as u8,
                (x * 17 + y * 3) as u8,
            ]
        })
        .collect();
    assert_gamut_encode_libwebp_decode(&many, w, h, "many-color");
}
