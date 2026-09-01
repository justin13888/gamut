//! `PngEncoder::with_transparent_cleanup` (issue #224): rewriting the colour of invisible pixels.
//!
//! The claim has two halves and they need different techniques. That nothing *visible* changes is
//! a differential claim, checked by decoding with libpng and comparing every pixel a viewer could
//! see. That it actually pays is a size claim, checked against the same image encoded without it.
//!
//! Both halves matter: a cleanup that changed a visible pixel would be a correctness bug, and one
//! that saved no bytes would be churn.

mod common;

use gamut_core::{Dimensions, EncodeImage, ImageRef, Rgba8};
use gamut_png::{FilterStrategy, Level, PngEncoder};

const SIDE: u32 = 64;

fn encode(samples: &[u8], cleanup: bool, auto_reduce: bool) -> Vec<u8> {
    let dims = Dimensions::new(SIDE, SIDE).expect("valid dimensions");
    let image = ImageRef::<Rgba8>::new(samples, dims).expect("buffer matches dimensions");
    let mut out = Vec::new();
    PngEncoder::new()
        .with_compression(Level::Best)
        .with_filter(FilterStrategy::BruteForce)
        .with_auto_reduce(auto_reduce)
        .with_transparent_cleanup(cleanup)
        .encode_image(image, &mut out)
        .expect("encode");
    out
}

#[test]
fn every_visible_pixel_survives_cleanup_unchanged() {
    // libpng decodes both files; every pixel with a non-zero alpha must be byte-identical, and
    // every alpha must be identical everywhere. Only the colour under alpha == 0 may differ.
    let src = common::corpus::sprite_rgba(SIDE);
    let plain = libpng_oracle::decode_rgba8(&encode(&src, false, false)).2;
    let cleaned = libpng_oracle::decode_rgba8(&encode(&src, true, false)).2;

    assert_eq!(plain.len(), cleaned.len());
    let mut invisible_changed = 0usize;
    let (plain_px, _) = plain.as_chunks::<4>();
    let (clean_px, _) = cleaned.as_chunks::<4>();
    for (i, (a, b)) in plain_px.iter().zip(clean_px).enumerate() {
        assert_eq!(a[3], b[3], "pixel {i}: alpha must never change");
        if a[3] == 0 {
            if a[..3] != b[..3] {
                invisible_changed += 1;
            }
        } else {
            assert_eq!(a, b, "pixel {i} is visible and must be byte-identical");
        }
    }
    assert!(
        invisible_changed > 0,
        "the fixture must actually exercise the cleanup"
    );
}

#[test]
fn cleanup_shrinks_an_image_with_invisible_colour_noise() {
    let src = common::corpus::sprite_rgba(SIDE);
    let plain = encode(&src, false, false);
    let cleaned = encode(&src, true, false);
    assert!(
        cleaned.len() < plain.len(),
        "cleanup should pay on a sprite: {} vs {}",
        cleaned.len(),
        plain.len()
    );
}

#[test]
fn cleanup_is_inert_on_a_fully_opaque_image() {
    // No fully transparent pixel means nothing to rewrite, and the output must be byte-identical
    // rather than merely the same size — this is what pins that the pass is a no-op, not a
    // re-encode that happens to land on the same length.
    let src = common::corpus::flat_rgba(SIDE);
    assert_eq!(encode(&src, false, true), encode(&src, true, true));
}

#[test]
fn cleanup_collapses_invisible_pixels_into_one_palette_entry() {
    // The compounding effect: `analyze8` keys its palette on the whole RGBA quad, so invisible
    // pixels that differ only in unseen colour cost an entry each. This fixture has 64 visible
    // colours and 64 *distinct* invisible ones, which is over the 256-entry cliff only in the
    // sense that it doubles the table; cleaning collapses the invisible half.
    let mut src = vec![0u8; (SIDE * SIDE * 4) as usize];
    for (i, px) in src.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let v = (i % 64) as u8;
        if i % 2 == 0 {
            px.copy_from_slice(&[v, v, v, 255]);
        } else {
            // Invisible, and every one a different colour.
            px.copy_from_slice(&[v.wrapping_mul(3), v.wrapping_add(7), 200 - v, 0]);
        }
    }
    let plain = encode(&src, false, true);
    let cleaned = encode(&src, true, true);
    assert!(
        cleaned.len() < plain.len(),
        "collapsing the invisible half should shrink the palette: {} vs {}",
        cleaned.len(),
        plain.len()
    );
}

#[test]
fn cleanup_is_off_by_default() {
    // The default must stay byte-for-byte lossless, so an encoder that was never asked for
    // cleanup must produce exactly what it produced before this feature existed.
    let src = common::corpus::sprite_rgba(SIDE);
    let dims = Dimensions::new(SIDE, SIDE).expect("valid dimensions");
    let image = ImageRef::<Rgba8>::new(&src, dims).expect("buffer matches dimensions");
    let mut default_out = Vec::new();
    PngEncoder::new()
        .with_compression(Level::Best)
        .with_filter(FilterStrategy::BruteForce)
        .encode_image(image, &mut default_out)
        .expect("encode");
    assert_eq!(default_out, encode(&src, false, false));
}
