//! Differential conformance cross-checks against a vendored **libpng**.
//!
//! gamut ships no PNG decoder, so correctness is proven by decoding the encoder's output with libpng
//! and asserting the pixels (and IHDR fields) match the source exactly.

use gamut_core::{
    Bilevel, Dimensions, EncodeImage, Gray8, Gray16, GrayAlpha8, GrayAlpha16, ImageRef, Indexed8,
    Rgb8, Rgb16, Rgba8, Rgba16,
};
use gamut_deflate::Level;
use gamut_png::{FilterStrategy, FilterType, PngEncoder, PngPalette};

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
fn every_filter_strategy_round_trips() {
    let (w, h) = (32, 24);
    let src = rgb_pattern(w, h);
    let dims = Dimensions::new(w, h).unwrap();
    let strategies = [
        FilterStrategy::None,
        FilterStrategy::Fixed(FilterType::Sub),
        FilterStrategy::Fixed(FilterType::Up),
        FilterStrategy::Fixed(FilterType::Average),
        FilterStrategy::Fixed(FilterType::Paeth),
        FilterStrategy::MinSumAbs,
    ];
    for strategy in strategies {
        let mut png = Vec::new();
        PngEncoder::new()
            .with_filter(strategy)
            .encode_image(ImageRef::<Rgb8>::new(&src, dims).unwrap(), &mut png)
            .expect("encode");
        let dec = libpng_oracle::decode(&png);
        assert_eq!(dec.pixels, src, "{strategy:?}");
    }
}

#[test]
fn filtering_shrinks_a_gradient() {
    // A smooth gradient compresses much better filtered than unfiltered.
    let (w, h) = (64u32, 64u32);
    let mut src = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            src.push((x * 4) as u8);
            src.push((y * 4) as u8);
            src.push(((x + y) * 2) as u8);
        }
    }
    let dims = Dimensions::new(w, h).unwrap();
    let encode = |strategy| {
        let mut png = Vec::new();
        PngEncoder::new()
            .with_filter(strategy)
            .with_compression(Level::Best)
            .encode_image(ImageRef::<Rgb8>::new(&src, dims).unwrap(), &mut png)
            .expect("encode");
        png.len()
    };
    let unfiltered = encode(FilterStrategy::None);
    let filtered = encode(FilterStrategy::MinSumAbs);
    assert!(
        filtered < unfiltered,
        "filtered {filtered} should beat unfiltered {unfiltered}"
    );
}

/// Encodes an 8-bit image of pixel type `$ty` and asserts libpng decodes the exact source bytes
/// with the expected colour type.
macro_rules! check_8bit {
    ($ty:ty, $channels:expr, $color:expr) => {{
        let (w, h) = (20u32, 15u32);
        let n = (w * h) as usize * $channels;
        let src: Vec<u8> = (0..n)
            .map(|i| (i.wrapping_mul(37) ^ (i >> 2)) as u8)
            .collect();
        let mut png = Vec::new();
        PngEncoder::new()
            .with_compression(Level::Best)
            .encode_image(
                ImageRef::<$ty>::new(&src, Dimensions::new(w, h).unwrap()).unwrap(),
                &mut png,
            )
            .expect("encode");
        let dec = libpng_oracle::decode(&png);
        assert_eq!(dec.bit_depth, 8, "{}", stringify!($ty));
        assert_eq!(dec.color_type, $color, "{}", stringify!($ty));
        assert_eq!(dec.pixels, src, "{}", stringify!($ty));
    }};
}

/// As [`check_8bit`] but for 16-bit samples, comparing against the big-endian serialisation.
macro_rules! check_16bit {
    ($ty:ty, $channels:expr, $color:expr) => {{
        let (w, h) = (18u32, 13u32);
        let n = (w * h) as usize * $channels;
        let src: Vec<u16> = (0..n).map(|i| (i.wrapping_mul(1009)) as u16).collect();
        let mut png = Vec::new();
        PngEncoder::new()
            .with_compression(Level::Best)
            .encode_image(
                ImageRef::<$ty>::new(&src, Dimensions::new(w, h).unwrap()).unwrap(),
                &mut png,
            )
            .expect("encode");
        let dec = libpng_oracle::decode(&png);
        assert_eq!(dec.bit_depth, 16, "{}", stringify!($ty));
        assert_eq!(dec.color_type, $color, "{}", stringify!($ty));
        let expected: Vec<u8> = src.iter().flat_map(|s| s.to_be_bytes()).collect();
        assert_eq!(dec.pixels, expected, "{}", stringify!($ty));
    }};
}

#[test]
fn eight_bit_colour_types_round_trip() {
    check_8bit!(Gray8, 1, libpng_oracle::COLOR_GRAY);
    check_8bit!(GrayAlpha8, 2, libpng_oracle::COLOR_GRAY_ALPHA);
    check_8bit!(Rgb8, 3, libpng_oracle::COLOR_RGB);
    check_8bit!(Rgba8, 4, libpng_oracle::COLOR_RGBA);
}

#[test]
fn sixteen_bit_colour_types_round_trip() {
    check_16bit!(Gray16, 1, libpng_oracle::COLOR_GRAY);
    check_16bit!(GrayAlpha16, 2, libpng_oracle::COLOR_GRAY_ALPHA);
    check_16bit!(Rgb16, 3, libpng_oracle::COLOR_RGB);
    check_16bit!(Rgba16, 4, libpng_oracle::COLOR_RGBA);
}

#[test]
fn indexed8_with_palette_and_transparency_round_trips() {
    let (w, h) = (24u32, 18u32);
    let rgb: Vec<[u8; 3]> = vec![
        [10, 20, 30],
        [255, 0, 0],
        [0, 255, 0],
        [0, 0, 255],
        [128, 128, 128],
    ];
    let alpha = vec![0u8, 255, 128]; // entries 0/1/2 have alpha; 3/4 are opaque
    let palette = PngPalette::with_transparency(&rgb, &alpha).unwrap();
    let indices: Vec<u8> = (0..(w * h) as usize)
        .map(|i| (i % rgb.len()) as u8)
        .collect();

    let mut png = Vec::new();
    PngEncoder::new()
        .with_compression(Level::Best)
        .encode_indexed8(
            ImageRef::<Indexed8>::new(&indices, Dimensions::new(w, h).unwrap()).unwrap(),
            &palette,
            &mut png,
        )
        .expect("encode");

    // Raw decode: it is a palette image and the indices survive exactly. Five entries fit in a
    // 4-bit index, which the encoder selects automatically.
    let dec = libpng_oracle::decode(&png);
    assert_eq!(dec.color_type, libpng_oracle::COLOR_PALETTE);
    assert_eq!(dec.bit_depth, 4);
    assert_eq!(dec.pixels, indices);

    // Expanded decode: palette colours and tRNS resolve to the intended RGBA.
    let (dw, dh, rgba) = libpng_oracle::decode_rgba8(&png);
    assert_eq!((dw, dh), (w, h));
    let expected: Vec<u8> = indices
        .iter()
        .flat_map(|&idx| {
            let [r, g, b] = rgb[idx as usize];
            let a = alpha.get(idx as usize).copied().unwrap_or(255);
            [r, g, b, a]
        })
        .collect();
    assert_eq!(rgba, expected);
}

#[test]
fn indexed8_rejects_out_of_range_index() {
    let palette = PngPalette::new(&[[0, 0, 0], [255, 255, 255]]).unwrap();
    let indices = vec![0u8, 1, 2]; // 2 is out of range for a 2-entry palette
    let mut png = Vec::new();
    let result = PngEncoder::new().encode_indexed8(
        ImageRef::<Indexed8>::new(&indices, Dimensions::new(3, 1).unwrap()).unwrap(),
        &palette,
        &mut png,
    );
    assert!(result.is_err());
}

#[test]
fn indexed_uses_minimal_bit_depth() {
    // Palette size determines the smallest index bit depth; libpng must report it and recover the
    // indices (png_set_packing unpacks sub-byte indices to one byte each).
    for (entries, depth) in [(2usize, 1u8), (4, 2), (16, 4), (17, 8)] {
        let rgb: Vec<[u8; 3]> = (0..entries).map(|i| [i as u8, 0, 0]).collect();
        let palette = PngPalette::new(&rgb).unwrap();
        let (w, h) = (20u32, 8u32);
        let indices: Vec<u8> = (0..(w * h) as usize).map(|i| (i % entries) as u8).collect();
        let mut png = Vec::new();
        PngEncoder::new()
            .encode_indexed8(
                ImageRef::<Indexed8>::new(&indices, Dimensions::new(w, h).unwrap()).unwrap(),
                &palette,
                &mut png,
            )
            .expect("encode");
        let dec = libpng_oracle::decode(&png);
        assert_eq!(dec.color_type, libpng_oracle::COLOR_PALETTE, "{entries}");
        assert_eq!(dec.bit_depth, depth, "{entries} entries");
        assert_eq!(dec.pixels, indices, "{entries} entries");
    }
}

#[test]
fn bilevel_round_trips_as_1bit_gray() {
    // A non-byte-aligned width exercises sub-byte row padding.
    let (w, h) = (19u32, 7u32);
    let src: Vec<u8> = (0..(w * h) as usize)
        .map(|i| u8::from(i % 3 == 0) * 200)
        .collect();
    let mut png = Vec::new();
    PngEncoder::new()
        .with_compression(Level::Best)
        .encode_image(
            ImageRef::<Bilevel>::new(&src, Dimensions::new(w, h).unwrap()).unwrap(),
            &mut png,
        )
        .expect("encode");
    let dec = libpng_oracle::decode(&png);
    assert_eq!(dec.color_type, libpng_oracle::COLOR_GRAY);
    assert_eq!(dec.bit_depth, 1);
    let expected: Vec<u8> = src.iter().map(|&v| u8::from(v != 0)).collect();
    assert_eq!(dec.pixels, expected);
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
