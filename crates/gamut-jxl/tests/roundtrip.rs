//! End-to-end round-trip and conversion-contract tests for the pure-Rust decoder: gamut encodes a
//! stream with libjxl, gamut decodes it back with jxl-rs, and the result must be **bit-exact** to
//! the source (lossless) or obey the documented channel-conversion contracts.
//!
//! These exercise the encoder and decoder as a pair, so they need both codec halves; the module is
//! compiled only when both are available (i.e. off `wasm32`, with default features).
#![cfg(all(feature = "encode", feature = "decode", not(target_arch = "wasm32")))]

use gamut_core::{
    DecodeImage, Dimensions, EncodeImage, Gray8, Gray16, GrayAlpha8, GrayAlpha16, ImageBuf,
    ImageRef, Pixel, Rgb8, Rgb16, Rgba8, Rgba16,
};
use gamut_jxl::{Container, JxlDecoder, JxlEncoder};

/// The size grid: 1x1 up to a non-square textured image, including odd dimensions.
const SIZES: [(u32, u32); 5] = [(1, 1), (3, 7), (16, 16), (17, 13), (64, 100)];

/// A deterministic per-sample value in `0..=max`, non-flat so lossless is meaningfully exercised.
fn raw(x: u32, y: u32, c: u32, max: u32) -> u32 {
    let gradient = x.wrapping_mul(4).wrapping_add(y.wrapping_mul(3));
    let texture = ((x / 8) ^ (y / 8)).wrapping_mul(5);
    let channel = c.wrapping_mul(37);
    let base = gradient.wrapping_add(texture).wrapping_add(channel);
    let scale = if max > 0xFF { 251 } else { 1 };
    base.wrapping_mul(scale) & max
}

/// Generates interleaved 8-bit samples.
fn gen_u8(w: u32, h: u32, ch: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(w as usize * h as usize * ch);
    for y in 0..h {
        for x in 0..w {
            for c in 0..ch as u32 {
                v.push(raw(x, y, c, 0xFF) as u8);
            }
        }
    }
    v
}

/// Generates interleaved 16-bit samples.
fn gen_u16(w: u32, h: u32, ch: usize) -> Vec<u16> {
    let mut v = Vec::with_capacity(w as usize * h as usize * ch);
    for y in 0..h {
        for x in 0..w {
            for c in 0..ch as u32 {
                v.push(raw(x, y, c, 0xFFFF) as u16);
            }
        }
    }
    v
}

/// Emits a lossless round-trip test for one 8-bit layout, over the size grid and both containers.
macro_rules! roundtrip_u8 {
    ($name:ident, $pixel:ty) => {
        #[test]
        fn $name() {
            let ch = <$pixel as Pixel>::CHANNELS;
            for (w, h) in SIZES {
                for container in [Container::Codestream, Container::IsoBmff] {
                    let px = gen_u8(w, h, ch);
                    let dims = Dimensions::new(w, h).unwrap();
                    let img = ImageRef::<$pixel>::new(&px, dims).unwrap();
                    let bytes = JxlEncoder::lossless()
                        .with_container(container)
                        .encode_to_vec(img)
                        .unwrap();
                    let out: ImageBuf<$pixel> = JxlDecoder::new().decode_image(&bytes).unwrap();
                    assert_eq!(out.dimensions(), dims, "dims {w}x{h} {container:?}");
                    assert_eq!(
                        out.as_samples(),
                        px.as_slice(),
                        "lossless not bit-exact at {w}x{h} {container:?}"
                    );
                }
            }
        }
    };
}

/// Emits a lossless round-trip test for one 16-bit layout.
macro_rules! roundtrip_u16 {
    ($name:ident, $pixel:ty) => {
        #[test]
        fn $name() {
            let ch = <$pixel as Pixel>::CHANNELS;
            for (w, h) in SIZES {
                for container in [Container::Codestream, Container::IsoBmff] {
                    let px = gen_u16(w, h, ch);
                    let dims = Dimensions::new(w, h).unwrap();
                    let img = ImageRef::<$pixel>::new(&px, dims).unwrap();
                    let bytes = JxlEncoder::lossless()
                        .with_container(container)
                        .encode_to_vec(img)
                        .unwrap();
                    let out: ImageBuf<$pixel> = JxlDecoder::new().decode_image(&bytes).unwrap();
                    assert_eq!(out.dimensions(), dims, "dims {w}x{h} {container:?}");
                    assert_eq!(
                        out.as_samples(),
                        px.as_slice(),
                        "lossless not bit-exact at {w}x{h} {container:?}"
                    );
                }
            }
        }
    };
}

roundtrip_u8!(roundtrip_gray8, Gray8);
roundtrip_u8!(roundtrip_gray_alpha8, GrayAlpha8);
roundtrip_u8!(roundtrip_rgb8, Rgb8);
roundtrip_u8!(roundtrip_rgba8, Rgba8);
roundtrip_u16!(roundtrip_gray16, Gray16);
roundtrip_u16!(roundtrip_gray_alpha16, GrayAlpha16);
roundtrip_u16!(roundtrip_rgb16, Rgb16);
roundtrip_u16!(roundtrip_rgba16, Rgba16);

/// Encodes a lossless RGBA8 image and returns `(bytes, source pixels, dims)`.
fn encode_rgba8(w: u32, h: u32) -> (Vec<u8>, Vec<u8>, Dimensions) {
    let dims = Dimensions::new(w, h).unwrap();
    let px = gen_u8(w, h, Rgba8::CHANNELS);
    let img = ImageRef::<Rgba8>::new(&px, dims).unwrap();
    let bytes = JxlEncoder::lossless().encode_to_vec(img).unwrap();
    (bytes, px, dims)
}

#[test]
fn decode_image_into_reuses_allocation_when_dims_match() {
    let (bytes, px, dims) = encode_rgba8(17, 13);

    // Seed a destination of the matching geometry, remember its buffer identity, decode into it.
    let mut dst = ImageBuf::<Rgba8>::zeroed(dims).unwrap();
    let ptr_before = dst.as_samples().as_ptr();
    JxlDecoder::new()
        .decode_image_into(&bytes, &mut dst)
        .unwrap();
    let ptr_after = dst.as_samples().as_ptr();

    assert_eq!(dst.as_samples(), px.as_slice(), "decoded pixels correct");
    assert_eq!(
        ptr_before, ptr_after,
        "matching dimensions must reuse the destination's sample allocation"
    );
}

#[test]
fn decode_image_into_replaces_buffer_on_dimension_mismatch() {
    let (bytes, px, dims) = encode_rgba8(16, 16);

    // A destination of the wrong geometry must be replaced, and end up holding the decoded image.
    let mut dst = ImageBuf::<Rgba8>::zeroed(Dimensions::new(4, 4).unwrap()).unwrap();
    JxlDecoder::new()
        .decode_image_into(&bytes, &mut dst)
        .unwrap();
    assert_eq!(
        dst.dimensions(),
        dims,
        "destination resized to decoded dims"
    );
    assert_eq!(dst.as_samples(), px.as_slice(), "decoded pixels correct");
}

// ---- Conversion contracts: request a layout that differs from the stream's ----

#[test]
fn gray_stream_expands_to_rgb8_and_rgba8() {
    let (w, h) = (17, 13);
    let dims = Dimensions::new(w, h).unwrap();
    let gray = gen_u8(w, h, Gray8::CHANNELS);
    let img = ImageRef::<Gray8>::new(&gray, dims).unwrap();
    let bytes = JxlEncoder::lossless().encode_to_vec(img).unwrap();

    // Grayscale -> RGB8: each luminance replicated across R, G, B.
    let rgb: ImageBuf<Rgb8> = JxlDecoder::new().decode_image(&bytes).unwrap();
    assert_eq!(rgb.dimensions(), dims);
    for (i, &g) in gray.iter().enumerate() {
        assert_eq!(&rgb.as_samples()[i * 3..i * 3 + 3], &[g, g, g], "px {i}");
    }

    // Grayscale -> RGBA8: replicated colour plus a fully-opaque alpha.
    let rgba: ImageBuf<Rgba8> = JxlDecoder::new().decode_image(&bytes).unwrap();
    for (i, &g) in gray.iter().enumerate() {
        assert_eq!(
            &rgba.as_samples()[i * 4..i * 4 + 4],
            &[g, g, g, 0xFF],
            "px {i}"
        );
    }
}

#[test]
fn rgba_stream_decodes_as_rgb8_dropping_alpha() {
    let (bytes, px, dims) = encode_rgba8(16, 16);
    let rgb: ImageBuf<Rgb8> = JxlDecoder::new().decode_image(&bytes).unwrap();
    assert_eq!(rgb.dimensions(), dims);
    // Every RGB triple matches the source's colour channels; the source alpha is discarded.
    for i in 0..(16 * 16) {
        assert_eq!(
            &rgb.as_samples()[i * 3..i * 3 + 3],
            &px[i * 4..i * 4 + 3],
            "px {i}"
        );
    }
}

#[test]
fn opaque_rgba_stream_roundtrips_as_rgba8_bit_exact() {
    // An RGBA source whose alpha is entirely opaque must survive a lossless round-trip unchanged.
    let (w, h) = (16, 16);
    let dims = Dimensions::new(w, h).unwrap();
    let mut px = gen_u8(w, h, Rgba8::CHANNELS);
    for i in 0..(w * h) as usize {
        px[i * 4 + 3] = 0xFF; // force opaque alpha
    }
    let img = ImageRef::<Rgba8>::new(&px, dims).unwrap();
    let bytes = JxlEncoder::lossless().encode_to_vec(img).unwrap();
    let out: ImageBuf<Rgba8> = JxlDecoder::new().decode_image(&bytes).unwrap();
    assert_eq!(out.as_samples(), px.as_slice(), "opaque RGBA bit-exact");
}

#[test]
fn color_stream_requested_as_gray8_is_unsupported() {
    let (bytes, _px, _dims) = encode_rgba8(16, 16);
    let err = <JxlDecoder as DecodeImage<Gray8>>::decode_image(&JxlDecoder::new(), &bytes)
        .expect_err("a colour image cannot be decoded as grayscale");
    assert!(
        matches!(err, gamut_core::Error::Unsupported(_)),
        "expected Unsupported, got {err:?}"
    );

    // ...and the same holds for the RGB (no-alpha) colour source.
    let rgb_px = gen_u8(16, 16, Rgb8::CHANNELS);
    let dims = Dimensions::new(16, 16).unwrap();
    let rgb_bytes = JxlEncoder::lossless()
        .encode_to_vec(ImageRef::<Rgb8>::new(&rgb_px, dims).unwrap())
        .unwrap();
    let err = <JxlDecoder as DecodeImage<GrayAlpha8>>::decode_image(&JxlDecoder::new(), &rgb_bytes)
        .expect_err("colour as gray+alpha is also refused");
    assert!(matches!(err, gamut_core::Error::Unsupported(_)));
}

#[test]
fn sixteen_bit_stream_requested_as_rgb8_downscales() {
    // Decoding a 16-bit lossless stream into an 8-bit request exercises jxl-rs's bit-depth
    // reduction. jxl-rs does *not* truncate the high byte (`>> 8`); it renormalises through f32 and
    // rounds — the exact observed rule is `round(v / 65535 * 255)` (nearest, ties away from zero),
    // the natural full-range rescale. We assert that exact rule, and separately that it stays within
    // ±1 of a plain high-byte truncation (a coarse sanity bound).
    let (w, h) = (16, 16);
    let dims = Dimensions::new(w, h).unwrap();
    let px16 = gen_u16(w, h, Rgb16::CHANNELS);
    let img = ImageRef::<Rgb16>::new(&px16, dims).unwrap();
    let bytes = JxlEncoder::lossless().encode_to_vec(img).unwrap();

    let out8: ImageBuf<Rgb8> = JxlDecoder::new().decode_image(&bytes).unwrap();
    assert_eq!(out8.dimensions(), dims);
    for (i, &s16) in px16.iter().enumerate() {
        let got = out8.as_samples()[i];
        let exact = (f32::from(s16) / 65535.0 * 255.0).round() as u8;
        assert_eq!(
            got, exact,
            "16->8 rule mismatch at sample {i}: 0x{s16:04X} -> {got} (want {exact})"
        );
        // Sanity: never more than one code off a naive high-byte truncation.
        let truncated = (s16 >> 8) as u8;
        assert!(
            got.abs_diff(truncated) <= 1,
            "16->8 at sample {i}: {got} vs truncation {truncated} differ by >1"
        );
    }
}
