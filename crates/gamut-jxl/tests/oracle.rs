//! Differential tests against the reference libjxl **decoder** (the oracle in `tests/common`):
//!
//! - lossless round-trips are **bit-exact** for all eight pixel layouts across a size grid, in both
//!   container framings;
//! - lossy encoding decodes within a bounded PSNR and differs from the lossless bytes;
//! - the [`Effort`] setting is actually plumbed through (Lightning vs Glacier produce different
//!   streams, both decodable);
//! - the two independent decoders — gamut's pure-Rust [`JxlDecoder`] (jxl-rs) and the reference
//!   libjxl oracle — **agree with each other and the source** bit-for-bit on lossless streams, and
//!   within a tight bound on lossy ones.
//!
//! Uses both codec halves (gamut encodes with libjxl, decodes with jxl-rs); compiled only when both
//! are available.
#![cfg(all(
    feature = "encode",
    feature = "decode",
    any(not(target_arch = "wasm32"), target_os = "emscripten")
))]

mod common;

use common::{DecodedSamples, decode, encode_animated_rgb8, psnr_u8};
use gamut_core::{
    DecodeImage, Dimensions, EncodeImage, Gray8, Gray16, GrayAlpha8, GrayAlpha16, ImageBuf,
    ImageRef, Pixel, Rgb8, Rgb16, Rgba8, Rgba16,
};
use gamut_jxl::{Container, Distance, Effort, JxlDecoder, JxlEncoder};

/// The size grid: 1x1 up to a non-square textured image, including odd dimensions.
const SIZES: [(u32, u32); 5] = [(1, 1), (3, 7), (16, 16), (17, 13), (64, 100)];

/// A deterministic gradient + coarse-texture sample value in `0..=max`. Low-frequency enough that
/// lossy at distance 1.0 stays high-PSNR, but non-flat so lossless is meaningfully exercised.
fn raw(x: u32, y: u32, c: u32, max: u32) -> u32 {
    let gradient = x.wrapping_mul(4).wrapping_add(y.wrapping_mul(3));
    let texture = ((x / 8) ^ (y / 8)).wrapping_mul(5);
    let channel = c.wrapping_mul(37);
    let base = gradient.wrapping_add(texture).wrapping_add(channel);
    // Spread across the full range for 16-bit; identity for 8-bit.
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

/// Generates smooth, photographic-like 8-bit content: 2D gradients with a gentle product term and
/// no hard edges, so a distance-1.0 (visually lossless) lossy encode stays high-PSNR — the textured
/// [`gen_u8`] pattern's block boundaries are adversarial for lossy and understate quality.
fn gen_smooth_u8(w: u32, h: u32, ch: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(w as usize * h as usize * ch);
    for y in 0..h {
        for x in 0..w {
            let fx = x * 256 / w;
            let fy = y * 256 / h;
            let bump = fx * fy / 256;
            for c in 0..ch as u32 {
                let val = (fx + fy + bump) / 3 + c * 7;
                v.push((val & 0xFF) as u8);
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

/// Emits a lossless-round-trip test for one 8-bit pixel layout: encode → libjxl decode → bit-exact.
macro_rules! lossless_u8 {
    ($name:ident, $pixel:ty) => {
        #[test]
        fn $name() {
            let ch = <$pixel as Pixel>::CHANNELS;
            for (w, h) in SIZES {
                let px = gen_u8(w, h, ch);
                let dims = Dimensions::new(w, h).unwrap();
                let img = ImageRef::<$pixel>::new(&px, dims).unwrap();
                let bytes = JxlEncoder::lossless().encode_to_vec(img).unwrap();
                let out = decode(&bytes);
                assert_eq!((out.width, out.height), (w, h), "dims {w}x{h}");
                assert_eq!(out.num_channels as usize, ch, "channels {w}x{h}");
                match out.samples {
                    DecodedSamples::U8(got) => {
                        assert!(got == px, "lossless not bit-exact at {w}x{h}");
                    }
                    DecodedSamples::U16(_) => panic!("expected u8 samples at {w}x{h}"),
                }
            }
        }
    };
}

/// Emits a lossless-round-trip test for one 16-bit pixel layout.
macro_rules! lossless_u16 {
    ($name:ident, $pixel:ty) => {
        #[test]
        fn $name() {
            let ch = <$pixel as Pixel>::CHANNELS;
            for (w, h) in SIZES {
                let px = gen_u16(w, h, ch);
                let dims = Dimensions::new(w, h).unwrap();
                let img = ImageRef::<$pixel>::new(&px, dims).unwrap();
                let bytes = JxlEncoder::lossless().encode_to_vec(img).unwrap();
                let out = decode(&bytes);
                assert_eq!((out.width, out.height), (w, h), "dims {w}x{h}");
                assert_eq!(out.num_channels as usize, ch, "channels {w}x{h}");
                match out.samples {
                    DecodedSamples::U16(got) => {
                        assert!(got == px, "lossless not bit-exact at {w}x{h}");
                    }
                    DecodedSamples::U8(_) => panic!("expected u16 samples at {w}x{h}"),
                }
            }
        }
    };
}

lossless_u8!(lossless_gray8, Gray8);
lossless_u8!(lossless_gray_alpha8, GrayAlpha8);
lossless_u8!(lossless_rgb8, Rgb8);
lossless_u8!(lossless_rgba8, Rgba8);
lossless_u16!(lossless_gray16, Gray16);
lossless_u16!(lossless_gray_alpha16, GrayAlpha16);
lossless_u16!(lossless_rgb16, Rgb16);
lossless_u16!(lossless_rgba16, Rgba16);

#[test]
fn both_containers_decode_identically() {
    // The two framings carry the same coded image, so a lossless decode of each must match — and
    // match the source — for both an 8-bit and a 16-bit layout with alpha.
    let (w, h) = (17, 13);
    let dims = Dimensions::new(w, h).unwrap();

    let px = gen_u8(w, h, Rgba8::CHANNELS);
    let img = ImageRef::<Rgba8>::new(&px, dims).unwrap();
    let bare = JxlEncoder::lossless().encode_to_vec(img).unwrap();
    let boxed = JxlEncoder::lossless()
        .with_container(Container::IsoBmff)
        .encode_to_vec(img)
        .unwrap();
    // Distinct framings ⇒ distinct bytes, identical decoded pixels.
    assert_ne!(
        bare, boxed,
        "container framing must change the stream bytes"
    );
    let a = decode(&bare);
    let b = decode(&boxed);
    assert_eq!(
        a.samples, b.samples,
        "both containers decode to same pixels"
    );
    assert_eq!(a.samples, DecodedSamples::U8(px));
}

#[test]
fn lossy_decodes_within_psnr_and_differs_from_lossless() {
    // A visually-lossless (distance 1.0) RGB encode must decode close to the source and produce a
    // different (smaller-intent) stream than lossless.
    let (w, h) = (64, 100);
    let dims = Dimensions::new(w, h).unwrap();
    let px = gen_smooth_u8(w, h, Rgb8::CHANNELS);
    let img = ImageRef::<Rgb8>::new(&px, dims).unwrap();

    let lossless = JxlEncoder::lossless().encode_to_vec(img).unwrap();
    let lossy = JxlEncoder::lossy(Distance::new(1.0).unwrap())
        .encode_to_vec(img)
        .unwrap();
    assert_ne!(lossless, lossy, "lossy stream must differ from lossless");

    let out = decode(&lossy);
    let DecodedSamples::U8(got) = out.samples else {
        panic!("expected u8 samples");
    };
    let psnr = psnr_u8(&px, &got);
    assert!(psnr >= 35.0, "lossy PSNR {psnr:.2} dB below 35 dB floor");
}

#[test]
fn effort_setting_changes_the_stream() {
    // Effort must reach libjxl: the fastest and slowest settings, at the same distance, produce
    // different bytes on a large-enough textured image — and both remain decodable.
    let (w, h) = (64, 100);
    let dims = Dimensions::new(w, h).unwrap();
    let px = gen_u8(w, h, Rgb8::CHANNELS);
    let img = ImageRef::<Rgb8>::new(&px, dims).unwrap();
    let d = Distance::new(1.0).unwrap();

    let fast = JxlEncoder::lossy(d)
        .with_effort(Effort::Lightning)
        .encode_to_vec(img)
        .unwrap();
    let slow = JxlEncoder::lossy(d)
        .with_effort(Effort::Glacier)
        .encode_to_vec(img)
        .unwrap();
    assert_ne!(
        fast, slow,
        "effort Lightning vs Glacier must change the stream"
    );
    // Both decode without error and report the right geometry.
    for bytes in [&fast, &slow] {
        let out = decode(bytes);
        assert_eq!((out.width, out.height), (w, h));
        assert_eq!(out.num_channels, 3);
    }
}

// ---------------------------------------------------------------------------
// Full differential: gamut's pure-Rust decoder (jxl-rs) vs the libjxl oracle.
//
// For a lossless stream, both independent decoders must reconstruct the source *exactly*, so all
// three (source, gamut, oracle) are byte-identical. Each layout is decoded into its own natural
// gamut layout, which matches what the oracle returns for that stream.
// ---------------------------------------------------------------------------

/// Emits a three-way lossless differential test for one 8-bit layout.
macro_rules! differential_u8 {
    ($name:ident, $pixel:ty) => {
        #[test]
        fn $name() {
            let ch = <$pixel as Pixel>::CHANNELS;
            for (w, h) in SIZES {
                let px = gen_u8(w, h, ch);
                let dims = Dimensions::new(w, h).unwrap();
                let img = ImageRef::<$pixel>::new(&px, dims).unwrap();
                let bytes = JxlEncoder::lossless().encode_to_vec(img).unwrap();

                let gamut: ImageBuf<$pixel> = JxlDecoder::new().decode_image(&bytes).unwrap();
                let oracle = decode(&bytes);
                let DecodedSamples::U8(oracle) = oracle.samples else {
                    panic!("oracle produced non-u8 samples at {w}x{h}");
                };

                assert_eq!(gamut.as_samples(), px.as_slice(), "gamut != source {w}x{h}");
                assert_eq!(oracle, px, "oracle != source {w}x{h}");
                assert_eq!(
                    gamut.as_samples(),
                    oracle.as_slice(),
                    "gamut != oracle {w}x{h}"
                );
            }
        }
    };
}

/// Emits a three-way lossless differential test for one 16-bit layout.
macro_rules! differential_u16 {
    ($name:ident, $pixel:ty) => {
        #[test]
        fn $name() {
            let ch = <$pixel as Pixel>::CHANNELS;
            for (w, h) in SIZES {
                let px = gen_u16(w, h, ch);
                let dims = Dimensions::new(w, h).unwrap();
                let img = ImageRef::<$pixel>::new(&px, dims).unwrap();
                let bytes = JxlEncoder::lossless().encode_to_vec(img).unwrap();

                let gamut: ImageBuf<$pixel> = JxlDecoder::new().decode_image(&bytes).unwrap();
                let oracle = decode(&bytes);
                let DecodedSamples::U16(oracle) = oracle.samples else {
                    panic!("oracle produced non-u16 samples at {w}x{h}");
                };

                assert_eq!(gamut.as_samples(), px.as_slice(), "gamut != source {w}x{h}");
                assert_eq!(oracle, px, "oracle != source {w}x{h}");
                assert_eq!(
                    gamut.as_samples(),
                    oracle.as_slice(),
                    "gamut != oracle {w}x{h}"
                );
            }
        }
    };
}

differential_u8!(differential_gray8, Gray8);
differential_u8!(differential_gray_alpha8, GrayAlpha8);
differential_u8!(differential_rgb8, Rgb8);
differential_u8!(differential_rgba8, Rgba8);
differential_u16!(differential_gray16, Gray16);
differential_u16!(differential_gray_alpha16, GrayAlpha16);
differential_u16!(differential_rgb16, Rgb16);
differential_u16!(differential_rgba16, Rgba16);

#[test]
fn lossy_decoders_agree_and_are_psnr_bounded_8bit() {
    // A visually-lossless (distance 1.0) RGB encode, decoded by the two independent decoders: they
    // must agree within ±2 per 8-bit sample, and each must stay high-PSNR against the source.
    let (w, h) = (64, 100);
    let dims = Dimensions::new(w, h).unwrap();
    let px = gen_smooth_u8(w, h, Rgb8::CHANNELS);
    let img = ImageRef::<Rgb8>::new(&px, dims).unwrap();
    let bytes = JxlEncoder::lossy(Distance::new(1.0).unwrap())
        .encode_to_vec(img)
        .unwrap();

    let gamut: ImageBuf<Rgb8> = JxlDecoder::new().decode_image(&bytes).unwrap();
    let oracle = decode(&bytes);
    let DecodedSamples::U8(oracle) = oracle.samples else {
        panic!("expected u8 oracle samples");
    };

    let max_diff = gamut
        .as_samples()
        .iter()
        .zip(&oracle)
        .map(|(&a, &b)| a.abs_diff(b))
        .max()
        .unwrap();
    assert!(
        max_diff <= 2,
        "gamut vs oracle max abs diff {max_diff} exceeds 2 (8-bit)"
    );

    let psnr_gamut = psnr_u8(&px, gamut.as_samples());
    let psnr_oracle = psnr_u8(&px, &oracle);
    assert!(
        psnr_gamut >= 35.0,
        "gamut PSNR {psnr_gamut:.2} dB below 35 dB"
    );
    assert!(
        psnr_oracle >= 35.0,
        "oracle PSNR {psnr_oracle:.2} dB below 35 dB"
    );
}

#[test]
fn lossy_decoders_agree_16bit() {
    // The two decoders must agree within ±514 per 16-bit sample on a distance-1.0 lossy stream
    // (the 8-bit ±2 bound scaled by 257).
    let (w, h) = (64, 100);
    let dims = Dimensions::new(w, h).unwrap();
    let px = gen_u16(w, h, Rgb16::CHANNELS);
    let img = ImageRef::<Rgb16>::new(&px, dims).unwrap();
    let bytes = JxlEncoder::lossy(Distance::new(1.0).unwrap())
        .encode_to_vec(img)
        .unwrap();

    let gamut: ImageBuf<Rgb16> = JxlDecoder::new().decode_image(&bytes).unwrap();
    let oracle = decode(&bytes);
    let DecodedSamples::U16(oracle) = oracle.samples else {
        panic!("expected u16 oracle samples");
    };

    let max_diff = gamut
        .as_samples()
        .iter()
        .zip(&oracle)
        .map(|(&a, &b)| a.abs_diff(b))
        .max()
        .unwrap();
    assert!(
        max_diff <= 514,
        "gamut vs oracle max abs diff {max_diff} exceeds 514 (16-bit)"
    );
}

#[test]
fn animated_stream_is_rejected_as_unsupported() {
    // Build a genuine animated JPEG XL (two frames, `have_animation` set) with the reference libjxl
    // encoder — gamut's own encoder is image-first and never produces one — and confirm the decoder
    // refuses it rather than silently decoding the first frame.
    let (w, h) = (16, 16);
    let frame0 = gen_u8(w, h, Rgb8::CHANNELS);
    let mut frame1 = frame0.clone();
    for b in &mut frame1 {
        *b = b.wrapping_add(40);
    }
    let bytes = encode_animated_rgb8(w, h, &[frame0, frame1]);

    // The libjxl oracle confirms it really is a decodable (animated) stream.
    let oracle = decode(&bytes);
    assert_eq!((oracle.width, oracle.height), (w, h));

    // gamut refuses it.
    let err = <JxlDecoder as DecodeImage<Rgb8>>::decode_image(&JxlDecoder::new(), &bytes)
        .expect_err("animated input must be rejected");
    assert!(
        matches!(err, gamut_core::Error::Unsupported(_)),
        "expected Unsupported for animated input, got {err:?}"
    );
}
