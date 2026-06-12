//! Multi-page documents: tier-1 + libtiff cross-checks (P17).

use gamut_core::Dimensions;
use gamut_tiff::{Compression, TiffDecoder, TiffEncoder};

/// Distinct RGB pages of differing dimensions.
fn page(seed: u32, w: u32, h: u32) -> Vec<u8> {
    (0..w * h * 3)
        .map(|i| (i.wrapping_mul(31).wrapping_add(seed.wrapping_mul(97))) as u8)
        .collect()
}

fn pages() -> Vec<(Vec<u8>, u32, u32)> {
    vec![
        (page(0, 8, 6), 8, 6),
        (page(1, 17, 13), 17, 13),
        (page(2, 32, 5), 32, 5),
    ]
}

#[test]
fn multipage_roundtrips_in_gamut() {
    let p = pages();
    let refs: Vec<(&[u8], Dimensions)> = p
        .iter()
        .map(|(d, w, h)| {
            (
                d.as_slice(),
                Dimensions {
                    width: *w,
                    height: *h,
                },
            )
        })
        .collect();
    let mut tiff = Vec::new();
    TiffEncoder::new()
        .with_compression(Compression::Lzw)
        .encode_pages_rgb8(&refs, &mut tiff)
        .expect("encode");

    let dec = TiffDecoder::new();
    assert_eq!(dec.page_count(&tiff).expect("count"), p.len());
    for (i, (data, w, h)) in p.iter().enumerate() {
        let mut out = Vec::new();
        let dims = dec
            .decode_page_to_rgb8(&tiff, i, &mut out)
            .expect("decode page");
        assert_eq!((dims.width, dims.height), (*w, *h));
        assert_eq!(&out, data, "page {i}");
    }
    // Out-of-range page is rejected.
    assert!(
        dec.decode_page_to_rgb8(&tiff, p.len(), &mut Vec::new())
            .is_err()
    );
}

#[test]
fn gamut_multipage_is_decoded_by_libtiff() {
    let p = pages();
    let refs: Vec<(&[u8], Dimensions)> = p
        .iter()
        .map(|(d, w, h)| {
            (
                d.as_slice(),
                Dimensions {
                    width: *w,
                    height: *h,
                },
            )
        })
        .collect();
    let mut tiff = Vec::new();
    TiffEncoder::new()
        .encode_pages_rgb8(&refs, &mut tiff)
        .expect("encode");
    for (i, (data, w, h)) in p.iter().enumerate() {
        let dec = libtiff_oracle::decode_page(&tiff, i as u32).expect("libtiff decode page");
        assert_eq!((dec.width, dec.height, dec.samples_per_pixel), (*w, *h, 3));
        assert_eq!(&dec.pixels, data, "page {i}");
    }
}

#[test]
fn libtiff_multipage_is_decoded_by_gamut() {
    let p = pages();
    let refs: Vec<(&[u8], u32, u32)> = p.iter().map(|(d, w, h)| (d.as_slice(), *w, *h)).collect();
    let tiff = libtiff_oracle::encode_pages_rgb8(&refs, libtiff_oracle::Compression::Lzw)
        .expect("libtiff encode");
    let dec = TiffDecoder::new();
    assert_eq!(dec.page_count(&tiff).expect("count"), p.len());
    for (i, (data, _, _)) in p.iter().enumerate() {
        let mut out = Vec::new();
        dec.decode_page_to_rgb8(&tiff, i, &mut out)
            .expect("gamut decode page");
        assert_eq!(&out, data, "page {i}");
    }
}
