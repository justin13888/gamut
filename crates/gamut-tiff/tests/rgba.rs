//! RGBA images (4 samples, unassociated alpha): tier-1 + libtiff cross-checks (P13).

use gamut_core::{DecodeImage, Dimensions, EncodeImage, ImageBuf, ImageRef, Rgb8, Rgba8};
use gamut_tiff::{Compression, TiffDecoder, TiffEncoder};

const SIZES: &[(u32, u32)] = &[(1, 1), (3, 7), (17, 13), (64, 40)];

fn rgba_pattern(w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            v.push((x.wrapping_mul(5).wrapping_add(y)) as u8);
            v.push((y.wrapping_mul(7) ^ x) as u8);
            v.push((x.wrapping_add(y).wrapping_mul(3)) as u8);
            v.push((x.wrapping_mul(y)) as u8); // varied alpha
        }
    }
    v
}

#[test]
fn rgba_roundtrips_in_gamut() {
    for &comp in &[Compression::None, Compression::PackBits, Compression::Lzw] {
        for &(w, h) in SIZES {
            let src = rgba_pattern(w, h);
            let mut tiff = Vec::new();
            TiffEncoder::new()
                .with_compression(comp)
                .encode_image(
                    ImageRef::<Rgba8>::new(
                        &src,
                        Dimensions {
                            width: w,
                            height: h,
                        },
                    )
                    .unwrap(),
                    &mut tiff,
                )
                .expect("encode");
            let got: ImageBuf<Rgba8> = TiffDecoder::new().decode_image(&tiff).expect("decode");
            assert_eq!((got.dimensions().width, got.dimensions().height), (w, h));
            assert_eq!(got.as_samples(), src.as_slice(), "{comp:?} {w}x{h}");

            // RGB view drops alpha.
            let rgb: ImageBuf<Rgb8> = TiffDecoder::new().decode_image(&tiff).expect("rgb");
            let expect_rgb: Vec<u8> = src
                .chunks_exact(4)
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect();
            assert_eq!(rgb.as_samples(), expect_rgb.as_slice());
        }
    }
}

#[test]
fn gamut_rgba_is_decoded_by_libtiff() {
    for &(w, h) in SIZES {
        let src = rgba_pattern(w, h);
        let mut tiff = Vec::new();
        TiffEncoder::new()
            .with_compression(Compression::Lzw)
            .encode_image(
                ImageRef::<Rgba8>::new(
                    &src,
                    Dimensions {
                        width: w,
                        height: h,
                    },
                )
                .unwrap(),
                &mut tiff,
            )
            .expect("encode");
        let dec = libtiff_oracle::decode_tiff(&tiff).expect("libtiff decode");
        assert_eq!((dec.width, dec.height, dec.samples_per_pixel), (w, h, 4));
        assert_eq!(dec.pixels, src, "{w}x{h}");
    }
}

#[test]
fn libtiff_rgba_is_decoded_by_gamut() {
    for &(w, h) in SIZES {
        let src = rgba_pattern(w, h);
        let tiff = libtiff_oracle::encode_rgba8(&src, w, h, libtiff_oracle::Compression::Lzw)
            .expect("libtiff encode");
        let got: ImageBuf<Rgba8> = TiffDecoder::new()
            .decode_image(&tiff)
            .expect("gamut decode");
        assert_eq!(got.as_samples(), src.as_slice(), "{w}x{h}");
    }
}
