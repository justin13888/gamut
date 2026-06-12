//! LZW compression: tier-1 round-trips and libtiff cross-checks (P9).

use gamut_core::Dimensions;
use gamut_tiff::{Compression, TiffDecoder, TiffEncoder};

const SIZES: &[(u32, u32)] = &[(1, 1), (3, 7), (17, 13), (64, 100), (130, 9)];

fn rgb_pattern(w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            let band = (x / 4) as u8;
            v.push(band.wrapping_mul(20));
            v.push((y as u8).wrapping_mul(3));
            v.push(if x % 9 == 0 { 255 } else { band });
        }
    }
    v
}

fn gray_pattern(w: u32, h: u32) -> Vec<u8> {
    (0..w * h).map(|i| ((i / 3) % 7) as u8 * 30).collect()
}

#[test]
fn lzw_rgb_roundtrips_in_gamut() {
    for &(w, h) in SIZES {
        let src = rgb_pattern(w, h);
        let mut tiff = Vec::new();
        TiffEncoder::new()
            .with_compression(Compression::Lzw)
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
fn gamut_lzw_is_decoded_by_libtiff() {
    for &(w, h) in SIZES {
        let rgb = rgb_pattern(w, h);
        let mut tiff = Vec::new();
        TiffEncoder::new()
            .with_compression(Compression::Lzw)
            .encode_rgb8(
                &rgb,
                Dimensions {
                    width: w,
                    height: h,
                },
                &mut tiff,
            )
            .expect("encode");
        let dec = libtiff_oracle::decode_tiff(&tiff).expect("libtiff decode");
        assert_eq!((dec.width, dec.height, dec.samples_per_pixel), (w, h, 3));
        assert_eq!(dec.pixels, rgb, "rgb {w}x{h}");

        let gray = gray_pattern(w, h);
        let mut gtiff = Vec::new();
        TiffEncoder::new()
            .with_compression(Compression::Lzw)
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
fn libtiff_lzw_is_decoded_by_gamut() {
    use libtiff_oracle::Compression as OC;
    for &(w, h) in SIZES {
        let rgb = rgb_pattern(w, h);
        let tiff = libtiff_oracle::encode_rgb8(&rgb, w, h, OC::Lzw).expect("libtiff encode");
        let mut out = Vec::new();
        TiffDecoder::new()
            .decode_to_rgb8(&tiff, &mut out)
            .expect("gamut decode");
        assert_eq!(out, rgb, "rgb {w}x{h}");

        let gray = gray_pattern(w, h);
        let gtiff = libtiff_oracle::encode_gray8(&gray, w, h, OC::Lzw).expect("libtiff encode");
        let mut gout = Vec::new();
        TiffDecoder::new()
            .decode_to_gray8(&gtiff, &mut gout)
            .expect("gamut decode");
        assert_eq!(gout, gray, "gray {w}x{h}");
    }
}
