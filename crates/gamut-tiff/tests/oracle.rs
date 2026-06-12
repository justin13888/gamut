//! Differential conformance cross-checks against a vendored libtiff (P4).
//!
//! Two directions, pixel-exact for the lossless uncompressed path:
//!   A. gamut encodes → libtiff decodes back to the source pixels;
//!   B. libtiff encodes → gamut decodes back to the source pixels.

use gamut_core::{DecodeImage, Dimensions, EncodeImage, Gray8, ImageBuf, ImageRef, Rgb8};
use gamut_tiff::{TiffDecoder, TiffEncoder};
use libtiff_oracle::Compression;

const SIZES: &[(u32, u32)] = &[(1, 1), (3, 7), (16, 16), (17, 13), (64, 100), (100, 70)];

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

fn gray_pattern(w: u32, h: u32) -> Vec<u8> {
    (0..w * h)
        .map(|i| (i.wrapping_mul(97) >> 1) as u8)
        .collect()
}

#[test]
fn gamut_rgb_is_decoded_by_libtiff() {
    for &(w, h) in SIZES {
        let src = rgb_pattern(w, h);
        let mut tiff = Vec::new();
        TiffEncoder::new()
            .encode_image(
                ImageRef::<Rgb8>::new(
                    &src,
                    Dimensions {
                        width: w,
                        height: h,
                    },
                )
                .unwrap(),
                &mut tiff,
            )
            .expect("gamut encode");
        let dec = libtiff_oracle::decode_tiff(&tiff).expect("libtiff decode");
        assert_eq!((dec.width, dec.height, dec.samples_per_pixel), (w, h, 3));
        assert_eq!(dec.pixels, src, "RGB mismatch at {w}x{h}");
    }
}

#[test]
fn gamut_gray_is_decoded_by_libtiff() {
    for &(w, h) in SIZES {
        let src = gray_pattern(w, h);
        let mut tiff = Vec::new();
        TiffEncoder::new()
            .encode_image(
                ImageRef::<Gray8>::new(
                    &src,
                    Dimensions {
                        width: w,
                        height: h,
                    },
                )
                .unwrap(),
                &mut tiff,
            )
            .expect("gamut encode");
        let dec = libtiff_oracle::decode_tiff(&tiff).expect("libtiff decode");
        assert_eq!((dec.width, dec.height, dec.samples_per_pixel), (w, h, 1));
        assert_eq!(dec.pixels, src, "gray mismatch at {w}x{h}");
    }
}

#[test]
fn libtiff_rgb_is_decoded_by_gamut() {
    for &(w, h) in SIZES {
        let src = rgb_pattern(w, h);
        let tiff =
            libtiff_oracle::encode_rgb8(&src, w, h, Compression::None).expect("libtiff encode");
        let got: ImageBuf<Rgb8> = TiffDecoder::new()
            .decode_image(&tiff)
            .expect("gamut decode");
        assert_eq!((got.dimensions().width, got.dimensions().height), (w, h));
        assert_eq!(got.as_samples(), src.as_slice(), "RGB mismatch at {w}x{h}");
    }
}

#[test]
fn libtiff_gray_is_decoded_by_gamut() {
    for &(w, h) in SIZES {
        let src = gray_pattern(w, h);
        let tiff =
            libtiff_oracle::encode_gray8(&src, w, h, Compression::None).expect("libtiff encode");
        let got: ImageBuf<Gray8> = TiffDecoder::new()
            .decode_image(&tiff)
            .expect("gamut decode");
        assert_eq!((got.dimensions().width, got.dimensions().height), (w, h));
        assert_eq!(got.as_samples(), src.as_slice(), "gray mismatch at {w}x{h}");
    }
}
