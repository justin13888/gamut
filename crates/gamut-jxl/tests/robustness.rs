//! Hostile-input corpus for the pure-Rust decoder: on empty, truncated, garbage, bit-flipped and
//! over-large input the decoder must return an `Err` and **never panic** or allocate unboundedly.
//!
//! The invariant everywhere is *no panic*. A `#[test]` that panics fails, so simply driving the
//! decoder over each input (and, where meaningful, asserting the `Err`) is the whole test — we do not
//! need `catch_unwind`.
//!
//! The file itself needs only the decoder, so it is gated on `decode`; the parts that first *produce*
//! a valid stream to mangle need the libjxl-backed encoder and so are gated additionally on
//! `all(feature = "encode", not(target_arch = "wasm32"))`, exactly like the other differential tests.
#![cfg(feature = "decode")]

use gamut_core::{DecodeImage, Error, Rgba8};
use gamut_jxl::JxlDecoder;

// Only the stream-producing corpus needs the shared helpers (and the libjxl-backed encoder they use),
// so the module is pulled in under the same gate. `mod common;` here resolves to `tests/common/mod.rs`.
#[cfg(all(feature = "encode", not(target_arch = "wasm32")))]
mod common;

/// Decodes `data` as `Rgba8` — the widest natural request, so the internal conversion paths (alpha
/// padding, grayscale expansion) are exercised on whatever the (possibly corrupt) header claims.
fn decode_rgba8(data: &[u8]) -> gamut_core::Result<gamut_core::ImageBuf<Rgba8>> {
    <JxlDecoder as DecodeImage<Rgba8>>::decode_image(&JxlDecoder::new(), data)
}

/// A short deterministic "random-ish" junk generator: a linear congruential sweep, so the garbage is
/// reproducible across runs (no `rand` dependency, no flakiness).
fn junk(len: usize, seed: u32) -> Vec<u8> {
    let mut state = seed | 1;
    (0..len)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 24) as u8
        })
        .collect()
}

#[test]
fn empty_input_errors_without_panicking() {
    assert!(decode_rgba8(&[]).is_err(), "empty input must error");
}

#[test]
fn bare_signature_alone_errors() {
    // The 2-byte codestream signature with nothing after it is not a decodable image.
    assert!(decode_rgba8(&[0xFF, 0x0A]).is_err());
}

#[test]
fn signatures_followed_by_garbage_error() {
    // A valid *signature* followed by a garbage body must be rejected, not decoded and not panic.
    for seed in [1u32, 0xDEAD_BEEF, 0x1234_5678] {
        // Bare codestream signature 0xFF 0x0A + junk.
        let mut cs = vec![0xFF, 0x0A];
        cs.extend_from_slice(&junk(512, seed));
        assert!(decode_rgba8(&cs).is_err(), "FF0A+junk seed {seed:#x}");

        // The 12-byte ISO BMFF JXL signature box + junk.
        const JXL_BOX: [u8; 12] = [
            0x00, 0x00, 0x00, 0x0C, 0x4A, 0x58, 0x4C, 0x20, 0x0D, 0x0A, 0x87, 0x0A,
        ];
        let mut ct = JXL_BOX.to_vec();
        ct.extend_from_slice(&junk(512, seed ^ 0xFFFF));
        assert!(decode_rgba8(&ct).is_err(), "box+junk seed {seed:#x}");
    }
}

/// The stream-producing corpus: everything that first encodes a real JPEG XL stream (via libjxl) and
/// then feeds a mangled version of it to the decoder.
#[cfg(all(feature = "encode", not(target_arch = "wasm32")))]
mod with_streams {
    use gamut_core::{Dimensions, EncodeImage, Gray8, ImageRef, Pixel, Rgb8};
    use gamut_jxl::{Container, Effort, JxlEncoder};

    use super::{Error, decode_rgba8};
    use crate::common::gen_u8;

    /// A small, valid, lossless bare codestream (textured 16×16 RGB).
    fn valid_codestream() -> Vec<u8> {
        let (w, h) = (16u32, 16u32);
        let px = gen_u8(w, h, Rgb8::CHANNELS);
        let dims = Dimensions::new(w, h).unwrap();
        let img = ImageRef::<Rgb8>::new(&px, dims).unwrap();
        JxlEncoder::lossless().encode_to_vec(img).unwrap()
    }

    /// The same image, in the ISO BMFF container framing.
    fn valid_container() -> Vec<u8> {
        let (w, h) = (16u32, 16u32);
        let px = gen_u8(w, h, Rgb8::CHANNELS);
        let dims = Dimensions::new(w, h).unwrap();
        let img = ImageRef::<Rgb8>::new(&px, dims).unwrap();
        JxlEncoder::lossless()
            .with_container(Container::IsoBmff)
            .encode_to_vec(img)
            .unwrap()
    }

    #[test]
    fn short_prefixes_of_both_framings_error() {
        // A prefix of 1..=16 bytes of either framing is far too short to be a complete image, so
        // every one must return an error (and none may panic).
        let cs = valid_codestream();
        let ct = valid_container();
        for stream in [&cs, &ct] {
            for len in 1..=16usize.min(stream.len()) {
                assert!(
                    decode_rgba8(&stream[..len]).is_err(),
                    "prefix len {len} decoded unexpectedly"
                );
            }
        }
    }

    #[test]
    fn systematic_truncations_error_until_full() {
        // Every 7th truncation length from 0 up to (but not including) the full stream must error:
        // jxl-rs signals the missing tail as "needs more input", which the decoder maps to a
        // truncation error. The full stream itself decodes cleanly.
        for stream in [valid_codestream(), valid_container()] {
            let full = stream.len();
            let mut len = 0;
            while len < full {
                assert!(
                    decode_rgba8(&stream[..len]).is_err(),
                    "truncation to {len}/{full} bytes decoded unexpectedly"
                );
                len += 7;
            }
            // Sanity: the untruncated stream is genuinely decodable, so the errors above are about
            // truncation, not an always-broken stream.
            assert!(decode_rgba8(&stream).is_ok(), "full stream must decode");
        }
    }

    #[test]
    fn single_bit_flips_never_panic() {
        // Flip each bit in the first 256 bytes (or the whole stream if shorter) of a small valid
        // codestream and decode the result. Bits in the header usually corrupt the stream (Err), but
        // bits in the entropy-coded sample data may decode to a *different* image — both outcomes are
        // acceptable. The invariant is only: no panic, and no unbounded allocation (the decoder's
        // pixel limit bounds any size claimed by a flipped dimension field).
        let base = valid_codestream();
        let scan = base.len().min(256);
        for byte in 0..scan {
            for bit in 0..8u8 {
                let mut m = base.clone();
                m[byte] ^= 1 << bit;
                // Drive the decoder; discard the outcome. A panic here fails the test.
                let _ = decode_rgba8(&m);
            }
        }
    }

    #[test]
    fn oversized_dimensions_trigger_the_pixel_limit() {
        // Honestly provoke the decoder's pixel-limit guard: encode a real 10000×10000 grayscale image
        // (flat, so it compresses to a few KiB and encodes in well under a second), then decode it.
        // jxl-rs checks `xsize.max(16) * ysize * (3 + extra_channels)` against the decoder's
        // `1 << 28` pixel limit while parsing the file header — 10000·10000·3 = 3.0e8 ≥ 2.68e8 — so it
        // rejects the stream *before* allocating any pixel buffer. This exercises the
        // `ImageSizeTooLarge → pixel-limit` mapping on a genuine oversized stream, not a forged header.
        let (w, h) = (10_000u32, 10_000u32);
        let px = vec![0u8; w as usize * h as usize];
        let dims = Dimensions::new(w, h).unwrap();
        let img = ImageRef::<Gray8>::new(&px, dims).unwrap();
        let bytes = JxlEncoder::lossless()
            .with_effort(Effort::Lightning)
            .encode_to_vec(img)
            .unwrap();

        let err = decode_rgba8(&bytes).expect_err("oversized image must be rejected");
        assert!(
            matches!(
                err,
                Error::InvalidInput("JXL: image exceeds the decoder pixel limit")
            ),
            "expected the pixel-limit error, got {err:?}"
        );
    }
}
