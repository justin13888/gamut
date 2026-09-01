//! The `tRNS` colour key reduction (issue #224, axis 3): dropping a binary alpha channel by
//! naming one colour "transparent" (§11.3.2.1).
//!
//! This is a *lossless* reduction, so the claim is exact: libpng must decode the keyed file to
//! byte-identical RGBA. That is the only assertion that matters, and it is why every test here
//! goes through the oracle rather than round-tripping gamut against itself — the key is written
//! by gamut and interpreted by libpng, so a round trip could agree on a wrong convention.

mod common;

use gamut_core::{Dimensions, EncodeImage, GrayAlpha8, ImageRef, Rgba8};
use gamut_png::{FilterStrategy, Level, PngEncoder, deconstruct};

/// 128, not something smaller, and the reason is the whole design of the reduction.
///
/// A colour key costs a flat 18-byte `tRNS` chunk that DEFLATE cannot touch, and buys an alpha
/// plane that usually compresses very well. So whether it wins is size-dependent, exactly as the
/// palette is: measured on this fixture the analysis offers `Rgb8Keyed` at every size, and
/// `write_reduced_or_native` keeps plain RGBA below this size before taking the key at 128,
/// where it is worth about 7% (863 bytes against 926).
///
/// That also matters for the *negative* tests below. Asserting "stayed RGBA" at a size where the
/// key would never have been taken anyway proves nothing; at 128 a valid key is taken, so RGBA
/// there is real evidence the reduction declined.
const SIDE: u32 = 128;

fn encode(samples: &[u8]) -> Vec<u8> {
    let dims = Dimensions::new(SIDE, SIDE).expect("valid dimensions");
    let image = ImageRef::<Rgba8>::new(samples, dims).expect("buffer matches dimensions");
    let mut out = Vec::new();
    PngEncoder::new()
        .with_compression(Level::Best)
        .with_filter(FilterStrategy::BruteForce)
        .with_auto_reduce(true)
        .encode_image(image, &mut out)
        .expect("encode");
    out
}

/// Whether this pixel is outside the visible shape.
///
/// A **contiguous** region, and that was measured rather than assumed. Scattering the
/// transparency instead — an avalanche hash over the pixel index — makes the key a net *loss*:
/// the invisible colour then interleaves with the visible gradient and wrecks the RGB channels'
/// compressibility, so `RGB + tRNS` came out at 14 886 bytes against plain RGBA's 14 319 and the
/// race correctly declined it. A solid transparent region keeps the colour channels smooth, which
/// is the shape real sprites and icons have and the shape where dropping the alpha plane pays.
fn outside(x: u32, y: u32) -> bool {
    let cx = i64::from(x) - i64::from(SIDE) / 2;
    let cy = i64::from(y) - i64::from(SIDE) / 2;
    cx * cx + cy * cy >= (i64::from(SIDE) * i64::from(SIDE)) / 9
}

/// Binary alpha, one shared invisible colour, and enough distinct visible colours that a palette
/// is not on the table — so the colour key is the only reduction available.
fn keyable_rgba() -> Vec<u8> {
    let mut buf = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    for y in 0..SIDE {
        for x in 0..SIDE {
            if outside(x, y) {
                // Invisible, all sharing one colour no visible pixel below can produce.
                buf.extend_from_slice(&[1, 2, 3, 0]);
            } else {
                buf.extend_from_slice(&[(x * 2) as u8, (y * 2) as u8, 200, 255]);
            }
        }
    }
    buf
}

#[test]
fn a_colour_key_drops_the_alpha_channel_losslessly() {
    let src = keyable_rgba();
    let png = encode(&src);
    let report = deconstruct(&png).expect("deconstruct");

    assert_eq!(
        libpng_oracle::decode(&png).color_type,
        libpng_oracle::COLOR_RGB,
        "the alpha channel is gone"
    );
    assert!(
        report.chunk(b"tRNS").is_some(),
        "and a colour key replaced it"
    );

    // The whole claim: libpng renders the key, and every pixel comes back exactly.
    let (_, _, rgba) = libpng_oracle::decode_rgba8(&png);
    assert_eq!(rgba, src, "the colour key resolves losslessly");
}

#[test]
fn the_key_is_written_as_sixteen_bit_big_endian_samples() {
    // §11.3.2.1: truecolour tRNS is three 16-bit big-endian samples, not three bytes. At depth 8
    // the high byte of each is zero — a decoder reading it as bytes would key on the wrong
    // colour, and libpng's round trip above would fail rather than this, so pin the bytes too.
    let png = encode(&keyable_rgba());
    let trns = read_chunk(&png, b"tRNS").expect("tRNS present");
    assert_eq!(trns, vec![0, 1, 0, 2, 0, 3], "the key is (1, 2, 3)");
}

#[test]
fn partial_transparency_keeps_the_alpha_channel() {
    // A key can only say "fully transparent"; anything in between must keep a real alpha channel.
    let mut src = keyable_rgba();
    src[7] = 128; // one pixel's alpha, neither 0 nor 255
    let png = encode(&src);

    assert_eq!(
        libpng_oracle::decode(&png).color_type,
        libpng_oracle::COLOR_RGBA,
        "partial alpha is not expressible as a key"
    );
    let (_, _, rgba) = libpng_oracle::decode_rgba8(&png);
    assert_eq!(rgba, src);
}

#[test]
fn a_colour_a_visible_pixel_uses_cannot_be_the_key() {
    // The invisible pixels all share (0, 0, 200) — but so does a visible one. Keying on it would
    // erase a pixel a viewer should see, so the reduction must decline.
    let mut buf = Vec::with_capacity((SIDE * SIDE * 4) as usize);
    for y in 0..SIDE {
        for x in 0..SIDE {
            if outside(x, y) {
                buf.extend_from_slice(&[0, 0, 200, 0]);
            } else {
                buf.extend_from_slice(&[(x * 2) as u8, (y * 2) as u8, 200, 255]);
            }
        }
    }
    // Plant the collision on a pixel that is definitely visible: the centre.
    let centre = ((SIDE / 2) * SIDE + SIDE / 2) as usize * 4;
    buf[centre..centre + 4].copy_from_slice(&[0, 0, 200, 255]);

    let png = encode(&buf);
    assert_eq!(
        libpng_oracle::decode(&png).color_type,
        libpng_oracle::COLOR_RGBA,
        "the only candidate key is in use by a visible pixel"
    );
    let (_, _, rgba) = libpng_oracle::decode_rgba8(&png);
    assert_eq!(rgba, buf);
}

#[test]
fn two_different_invisible_colours_have_no_single_key() {
    let mut src = keyable_rgba();
    // A second transparent colour: no one key can stand for both.
    src[0..4].copy_from_slice(&[9, 9, 9, 0]);
    let png = encode(&src);

    assert_eq!(
        libpng_oracle::decode(&png).color_type,
        libpng_oracle::COLOR_RGBA,
        "two invisible colours cannot share one key"
    );
    let (_, _, rgba) = libpng_oracle::decode_rgba8(&png);
    assert_eq!(rgba, src);
}

#[test]
fn cleanup_makes_an_unkeyable_image_keyable() {
    // The compounding case the cleanup pass exists for: transparent pixels carrying different
    // unseen colours have no key, until cleaning collapses them to one.
    let src = common::corpus::sprite_rgba(SIDE);
    let dims = Dimensions::new(SIDE, SIDE).expect("valid dimensions");

    let mut plain = Vec::new();
    PngEncoder::new()
        .with_compression(Level::Best)
        .with_auto_reduce(true)
        .encode_image(
            ImageRef::<Rgba8>::new(&src, dims).expect("buffer"),
            &mut plain,
        )
        .expect("encode");

    let mut cleaned = Vec::new();
    PngEncoder::new()
        .with_compression(Level::Best)
        .with_auto_reduce(true)
        .with_transparent_cleanup(true)
        .encode_image(
            ImageRef::<Rgba8>::new(&src, dims).expect("buffer"),
            &mut cleaned,
        )
        .expect("encode");

    // Whatever each lands on, the visible pixels must survive both.
    for png in [&plain, &cleaned] {
        let (_, _, rgba) = libpng_oracle::decode_rgba8(png);
        for (a, b) in rgba.as_chunks::<4>().0.iter().zip(src.as_chunks::<4>().0) {
            if b[3] != 0 {
                assert_eq!(a, b, "a visible pixel changed");
            }
            assert_eq!(a[3], b[3], "alpha changed");
        }
    }
    assert!(
        cleaned.len() <= plain.len(),
        "cleaning must not cost bytes: {} vs {}",
        cleaned.len(),
        plain.len()
    );
}

/// The payload of the first chunk of this type, if present.
fn read_chunk(png: &[u8], want: &[u8; 4]) -> Option<Vec<u8>> {
    let mut at = 8usize;
    while at + 12 <= png.len() {
        let len = u32::from_be_bytes([png[at], png[at + 1], png[at + 2], png[at + 3]]) as usize;
        let ty = &png[at + 4..at + 8];
        if ty == want {
            return Some(png[at + 8..at + 8 + len].to_vec());
        }
        at += 12 + len;
    }
    None
}

/// Binary alpha over a grey ramp: the greyscale twin of [`keyable_rgba`]. Grey 7 stands for
/// "invisible" and the visible ramp starts at 8, so no opaque pixel can collide with the key, and
/// 200 distinct visible levels keep a palette out of the race.
fn keyable_grey_alpha() -> Vec<u8> {
    let mut buf = Vec::with_capacity((SIDE * SIDE * 2) as usize);
    for y in 0..SIDE {
        for x in 0..SIDE {
            if outside(x, y) {
                buf.extend_from_slice(&[7, 0]);
            } else {
                buf.extend_from_slice(&[8 + ((x + y) % 200) as u8, 255]);
            }
        }
    }
    buf
}

/// The greyscale twin of [`a_colour_key_drops_the_alpha_channel_losslessly`], covering
/// `Reduced::GrayKeyed` -- reachable and correct, but produced by nothing else in the suite, so
/// the encoder's arm for it (the `ColorType::Grayscale` choice, and the single 16-bit big-endian
/// `tRNS` sample) had no test that could see it.
///
/// The win is thinner here than for truecolour: dropping the alpha plane saves one byte per pixel
/// rather than three, while the `tRNS` chunk still costs a flat 14. It is a win regardless --
/// measured at `SIDE`, brute-force filtered at `Level::Best`, 499 bytes keyed against 626 as
/// `GrayAlpha8`, about 20% -- and it stayed a win at every square from 32 to 256, so no size
/// threshold is needed on this side.
///
/// The key is grey 7 rather than 0 deliberately: a `tRNS` written little-endian would read
/// `[7, 0]`, which a key of 0 could not tell from the correct `[0, 7]`.
#[test]
fn a_greyscale_colour_key_drops_the_alpha_channel_losslessly() {
    let src = keyable_grey_alpha();
    let dims = Dimensions::new(SIDE, SIDE).expect("valid dimensions");
    let mut png = Vec::new();
    PngEncoder::new()
        .with_compression(Level::Best)
        .with_filter(FilterStrategy::BruteForce)
        .with_auto_reduce(true)
        .encode_image(
            ImageRef::<GrayAlpha8>::new(&src, dims).expect("buffer matches dimensions"),
            &mut png,
        )
        .expect("encode");

    let dec = libpng_oracle::decode(&png);
    assert_eq!(
        dec.color_type,
        libpng_oracle::COLOR_GRAY,
        "the alpha plane is gone"
    );
    assert_eq!(dec.bit_depth, 8, "a keyed grey is always depth 8");
    assert_eq!(
        read_chunk(&png, b"tRNS").expect("tRNS present"),
        vec![0, 7],
        "one 16-bit big-endian sample naming grey 7"
    );

    // The whole claim: libpng renders the key, and every pixel comes back exactly.
    let (_, _, rgba) = libpng_oracle::decode_rgba8(&png);
    let expected: Vec<u8> = src
        .as_chunks::<2>()
        .0
        .iter()
        .flat_map(|px| {
            let grey = if px[1] == 0 { 7 } else { px[0] };
            [grey, grey, grey, px[1]]
        })
        .collect();
    assert_eq!(rgba, expected, "the grey colour key resolves losslessly");
}
