//! Modified Huffman (CCITT G3 1-D) bilevel compression: tier-1 + libtiff cross-checks (P8).

use gamut_core::Dimensions;
use gamut_tiff::{Compression, TiffDecoder, TiffEncoder};

// Includes widths > 64 (make-up codes) and not multiples of 8 (row bit-padding).
const SIZES: &[(u32, u32)] = &[(1, 1), (8, 3), (13, 9), (100, 7), (200, 16), (640, 4)];

/// A bilevel pattern (bytes 0/255) with runs of varied length, to exercise terminating + make-up
/// codes and the leading-black case.
fn pattern(w: u32, h: u32) -> Vec<u8> {
    let mut v = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            // Run boundaries depend on the row, and some rows start black.
            let white = ((x.wrapping_add(y)) / (3 + (y % 5))) % 2 == 0;
            v.push(if white { 255 } else { 0 });
        }
    }
    v
}

#[test]
fn mh_roundtrips_in_gamut() {
    for &(w, h) in SIZES {
        let src = pattern(w, h);
        let mut tiff = Vec::new();
        TiffEncoder::new()
            .with_compression(Compression::CcittRle)
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
fn gamut_mh_is_decoded_by_libtiff() {
    for &(w, h) in SIZES {
        let src = pattern(w, h);
        let mut tiff = Vec::new();
        TiffEncoder::new()
            .with_compression(Compression::CcittRle)
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
fn libtiff_mh_is_decoded_by_gamut() {
    for &(w, h) in SIZES {
        let src = pattern(w, h);
        let tiff =
            libtiff_oracle::encode_bilevel(&src, w, h, libtiff_oracle::Compression::CcittRle)
                .expect("libtiff encode");
        let mut out = Vec::new();
        TiffDecoder::new()
            .decode_to_gray8(&tiff, &mut out)
            .expect("gamut decode");
        assert_eq!(out, src, "{w}x{h}");
    }
}
