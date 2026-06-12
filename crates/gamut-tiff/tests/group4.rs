//! CCITT Group 4 (T.6) bilevel compression: tier-1 + libtiff cross-checks (P11).

use gamut_core::Dimensions;
use gamut_tiff::{Compression, TiffDecoder, TiffEncoder};

// Includes widths > 64 (make-up codes) and not multiples of 8 (row bit-padding).
const SIZES: &[(u32, u32)] = &[(1, 1), (8, 3), (13, 9), (100, 7), (200, 16), (640, 4)];

/// A bilevel pattern (0/255) where each row resembles the one above it (so vertical/pass modes
/// fire), with some rows starting black and varied run lengths.
fn pattern(w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            let shift = y % 4;
            let white = ((x + shift) / 5) % 2 == 0;
            v.push(if white { 255 } else { 0 });
        }
    }
    v
}

#[test]
fn g4_roundtrips_in_gamut() {
    for &(w, h) in SIZES {
        let src = pattern(w, h);
        let mut tiff = Vec::new();
        TiffEncoder::new()
            .with_compression(Compression::CcittGroup4Fax)
            .encode_bilevel(
                &src,
                Dimensions {
                    width: w,
                    height: h,
                },
                &mut tiff,
            )
            .expect("encode");
        let mut out = Vec::new();
        let dims = TiffDecoder::new()
            .decode_to_gray8(&tiff, &mut out)
            .expect("decode");
        assert_eq!((dims.width, dims.height), (w, h));
        assert_eq!(out, src, "{w}x{h}");
    }
}

#[test]
fn gamut_g4_is_decoded_by_libtiff() {
    for &(w, h) in SIZES {
        let src = pattern(w, h);
        let mut tiff = Vec::new();
        TiffEncoder::new()
            .with_compression(Compression::CcittGroup4Fax)
            .encode_bilevel(
                &src,
                Dimensions {
                    width: w,
                    height: h,
                },
                &mut tiff,
            )
            .expect("encode");
        let dec = libtiff_oracle::decode_tiff(&tiff).expect("libtiff decode");
        assert_eq!((dec.width, dec.height, dec.samples_per_pixel), (w, h, 1));
        assert_eq!(dec.pixels, src, "{w}x{h}");
    }
}

#[test]
fn libtiff_g4_is_decoded_by_gamut() {
    for &(w, h) in SIZES {
        let src = pattern(w, h);
        let tiff =
            libtiff_oracle::encode_bilevel(&src, w, h, libtiff_oracle::Compression::CcittGroup4Fax)
                .expect("libtiff encode");
        let mut out = Vec::new();
        TiffDecoder::new()
            .decode_to_gray8(&tiff, &mut out)
            .expect("gamut decode");
        assert_eq!(out, src, "{w}x{h}");
    }
}
