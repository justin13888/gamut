//! Differential conformance cross-checks against a vendored **libpng**.
//!
//! gamut ships no PNG decoder, so correctness is proven by decoding the encoder's output with libpng
//! and asserting the pixels (and IHDR fields) match the source exactly.

use gamut_core::{Dimensions, EncodeImage, ImageRef, Rgb8};
use gamut_deflate::Level;
use gamut_png::PngEncoder;

const SIZES: &[(u32, u32)] = &[
    (1, 1),
    (2, 2),
    (3, 7),
    (16, 16),
    (17, 13),
    (64, 100),
    (100, 70),
];

/// A deterministic RGB pattern with enough structure to exercise filtering and matching.
fn rgb_pattern(w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            v.push((x.wrapping_mul(31).wrapping_add(y)) as u8);
            v.push((y.wrapping_mul(17) ^ x) as u8);
            v.push((x.wrapping_add(y).wrapping_mul(5)) as u8);
        }
    }
    v
}

#[test]
fn gamut_rgb8_is_decoded_by_libpng() {
    for &(w, h) in SIZES {
        for level in [Level::Fast, Level::Default, Level::Best] {
            let src = rgb_pattern(w, h);
            let dims = Dimensions::new(w, h).unwrap();
            let mut png = Vec::new();
            PngEncoder::new()
                .with_compression(level)
                .encode_image(ImageRef::<Rgb8>::new(&src, dims).unwrap(), &mut png)
                .expect("encode");

            let dec = libpng_oracle::decode(&png);
            assert_eq!((dec.width, dec.height), (w, h), "dims {w}x{h} {level:?}");
            assert_eq!(
                dec.color_type,
                libpng_oracle::COLOR_RGB,
                "{w}x{h} {level:?}"
            );
            assert_eq!(dec.bit_depth, 8, "{w}x{h} {level:?}");
            assert_eq!(dec.rowbytes, w as usize * 3, "{w}x{h} {level:?}");
            assert_eq!(dec.pixels, src, "pixels {w}x{h} {level:?}");
        }
    }
}

#[test]
fn solid_image_round_trips() {
    // A flat colour is the highly-compressible extreme; libpng must still recover it exactly.
    let (w, h) = (40, 30);
    let src = vec![0x7Fu8; (w * h * 3) as usize];
    let mut png = Vec::new();
    PngEncoder::new()
        .with_compression(Level::Best)
        .encode_image(
            ImageRef::<Rgb8>::new(&src, Dimensions::new(w, h).unwrap()).unwrap(),
            &mut png,
        )
        .expect("encode");
    let dec = libpng_oracle::decode(&png);
    assert_eq!(dec.pixels, src);
}
