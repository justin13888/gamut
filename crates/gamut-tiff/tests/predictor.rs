//! Horizontal differencing predictor (LZW + Predictor=2): tier-1 + libtiff cross-checks (P10).

use gamut_core::Dimensions;
use gamut_tiff::{Compression, Predictor, TiffDecoder, TiffEncoder};

const SIZES: &[(u32, u32)] = &[(1, 1), (3, 7), (17, 13), (64, 40), (200, 9)];

/// A smooth gradient — the kind of content horizontal differencing helps — with a little texture.
fn rgb_pattern(w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            v.push((x.wrapping_mul(2).wrapping_add(y)) as u8);
            v.push((x.wrapping_add(y.wrapping_mul(2))) as u8);
            v.push((128 + (x as i32 - y as i32)) as u8);
        }
    }
    v
}

fn gray_pattern(w: u32, h: u32) -> Vec<u8> {
    (0..w * h).map(|i| (i / 2) as u8).collect()
}

#[test]
fn predictor_rgb_roundtrips_in_gamut() {
    for &(w, h) in SIZES {
        let src = rgb_pattern(w, h);
        let mut tiff = Vec::new();
        TiffEncoder::new()
            .with_compression(Compression::Lzw)
            .with_predictor(Predictor::HorizontalDifferencing)
            .encode_rgb8(
                &src,
                Dimensions {
                    width: w,
                    height: h,
                },
                &mut tiff,
            )
            .expect("encode");
        let mut out = Vec::new();
        TiffDecoder::new()
            .decode_to_rgb8(&tiff, &mut out)
            .expect("decode");
        assert_eq!(out, src, "{w}x{h}");
    }
}

#[test]
fn gamut_predictor_is_decoded_by_libtiff() {
    for &(w, h) in SIZES {
        let rgb = rgb_pattern(w, h);
        let mut tiff = Vec::new();
        TiffEncoder::new()
            .with_compression(Compression::Lzw)
            .with_predictor(Predictor::HorizontalDifferencing)
            .encode_rgb8(
                &rgb,
                Dimensions {
                    width: w,
                    height: h,
                },
                &mut tiff,
            )
            .expect("encode");
        // libtiff reverses the predictor on read, so the decoded samples must match the source.
        let dec = libtiff_oracle::decode_tiff(&tiff).expect("libtiff decode");
        assert_eq!(dec.pixels, rgb, "rgb {w}x{h}");

        let gray = gray_pattern(w, h);
        let mut gtiff = Vec::new();
        TiffEncoder::new()
            .with_compression(Compression::Lzw)
            .with_predictor(Predictor::HorizontalDifferencing)
            .encode_gray8(
                &gray,
                Dimensions {
                    width: w,
                    height: h,
                },
                &mut gtiff,
            )
            .expect("encode");
        assert_eq!(
            libtiff_oracle::decode_tiff(&gtiff).expect("decode").pixels,
            gray
        );
    }
}

#[test]
fn libtiff_predictor_is_decoded_by_gamut() {
    use libtiff_oracle::Compression as OC;
    for &(w, h) in SIZES {
        let rgb = rgb_pattern(w, h);
        let tiff =
            libtiff_oracle::encode_rgb8_predictor(&rgb, w, h, OC::Lzw).expect("libtiff encode");
        let mut out = Vec::new();
        TiffDecoder::new()
            .decode_to_rgb8(&tiff, &mut out)
            .expect("gamut decode");
        assert_eq!(out, rgb, "rgb {w}x{h}");

        let gray = gray_pattern(w, h);
        let gtiff =
            libtiff_oracle::encode_gray8_predictor(&gray, w, h, OC::Lzw).expect("libtiff encode");
        let mut gout = Vec::new();
        TiffDecoder::new()
            .decode_to_gray8(&gtiff, &mut gout)
            .expect("gamut decode");
        assert_eq!(gout, gray, "gray {w}x{h}");
    }
}
