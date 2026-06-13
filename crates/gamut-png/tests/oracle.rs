//! Differential conformance cross-checks against a vendored **libpng**.
//!
//! gamut ships no PNG decoder, so correctness is proven by decoding the encoder's output with libpng
//! and asserting the pixels (and IHDR fields) match the source exactly.

use gamut_core::{
    Bilevel, Dimensions, EncodeImage, Gray8, Gray16, GrayAlpha8, GrayAlpha16, ImageRef, Indexed8,
    Rgb8, Rgb16, Rgba8, Rgba16,
};
use gamut_deflate::Level;
use gamut_png::{FilterStrategy, FilterType, PhysicalUnit, PngEncoder, PngPalette, SrgbIntent};

const SIZES: &[(u32, u32)] = &[
    (1, 1),
    (2, 2),
    (3, 7),
    (16, 16),
    (17, 13),
    (64, 100),
    (100, 70),
];

/// Whether a chunk of the given 4-byte type appears in the PNG stream.
fn contains_chunk(png: &[u8], ty: &[u8; 4]) -> bool {
    let mut i = 8; // skip the signature
    while i + 12 <= png.len() {
        let len = u32::from_be_bytes([png[i], png[i + 1], png[i + 2], png[i + 3]]) as usize;
        if &png[i + 4..i + 8] == ty {
            return true;
        }
        i += 12 + len;
    }
    false
}

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
fn ancillary_chunks_are_accepted_by_libpng() {
    // Pile on every standard ancillary chunk. libpng validates them on read (and decompresses
    // zTXt/iTXt internally), so if any chunk were malformed the oracle would abort. The pixels must
    // also survive unchanged.
    let (w, h) = (16u32, 16u32);
    let src = rgb_pattern(w, h);
    let dims = Dimensions::new(w, h).unwrap();
    let comment = "the quick brown fox ".repeat(20);
    let mut png = Vec::new();
    PngEncoder::new()
        .with_compression(Level::Best)
        .with_gamma(1.0 / 2.2)
        .with_srgb(SrgbIntent::Perceptual)
        .with_chromaticities((0.3127, 0.3290), (0.64, 0.33), (0.30, 0.60), (0.15, 0.06))
        .with_significant_bits(&[8, 8, 8])
        .with_background_rgb(0, 0, 0)
        .with_physical_dimensions(2835, 2835, PhysicalUnit::Meter)
        .with_time(2026, 6, 13, 1, 2, 3)
        .with_text("Title", "gamut")
        .with_compressed_text("Comment", &comment)
        .with_international_text("Author", "gämut")
        .encode_image(ImageRef::<Rgb8>::new(&src, dims).unwrap(), &mut png)
        .expect("encode");
    let dec = libpng_oracle::decode(&png);
    assert_eq!(dec.pixels, src);
}

#[test]
fn metadata_chunks_embed_and_image_survives() {
    let (w, h) = (12u32, 12u32);
    let src = rgb_pattern(w, h);
    let dims = Dimensions::new(w, h).unwrap();

    // Minimal-but-plausible EXIF (TIFF header + empty IFD) and ICC profile (132-byte header).
    let exif = [
        0x49, 0x49, 0x2A, 0x00, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    let mut icc = vec![0u8; 132];
    icc[0..4].copy_from_slice(&132u32.to_be_bytes()); // profile size
    icc[8..12].copy_from_slice(&0x0210_0000u32.to_be_bytes()); // version 2.1
    icc[12..16].copy_from_slice(b"mntr");
    icc[16..20].copy_from_slice(b"RGB ");
    icc[20..24].copy_from_slice(b"XYZ ");
    icc[36..40].copy_from_slice(b"acsp"); // ICC signature
    let xmp = r#"<?xpacket begin=""?><x:xmpmeta xmlns:x="adobe:ns:meta/"></x:xmpmeta><?xpacket end="r"?>"#;

    let mut png = Vec::new();
    PngEncoder::new()
        .with_compression(Level::Best)
        .with_exif(&exif)
        .with_icc_profile("test profile", &icc)
        .with_xmp(xmp)
        .encode_image(ImageRef::<Rgb8>::new(&src, dims).unwrap(), &mut png)
        .expect("encode");

    assert!(contains_chunk(&png, b"eXIf"), "eXIf present");
    assert!(contains_chunk(&png, b"iCCP"), "iCCP present");
    assert!(contains_chunk(&png, b"iTXt"), "iTXt (XMP) present");

    // libpng parses every chunk (decompressing iCCP); the image must survive unchanged.
    let dec = libpng_oracle::decode(&png);
    assert_eq!(dec.pixels, src);
}

#[test]
fn auto_reduce_is_lossless_smaller_and_picks_the_right_type() {
    let (w, h) = (32u32, 32u32);
    let n = (w * h) as usize;
    // Opaque greyscale stored as RGBA -> should reduce to greyscale.
    let gray: Vec<u8> = (0..n)
        .flat_map(|i| {
            let v = (i * 5) as u8;
            [v, v, v, 255]
        })
        .collect();
    // Three colours, one translucent -> palette + tRNS.
    let palette: Vec<u8> = (0..n)
        .flat_map(|i| match i % 3 {
            0 => [200, 0, 0, 255],
            1 => [0, 200, 0, 128],
            _ => [0, 0, 200, 255],
        })
        .collect();
    // Opaque, many distinct, non-grey -> drop the alpha channel (RGB).
    let mut opaque = Vec::with_capacity(n * 4);
    for y in 0..h {
        for x in 0..w {
            opaque.extend_from_slice(&[x as u8, y as u8, (x * y) as u8, 255]);
        }
    }

    let cases: [(&str, &Vec<u8>, u8); 3] = [
        ("gray", &gray, libpng_oracle::COLOR_GRAY),
        ("palette", &palette, libpng_oracle::COLOR_PALETTE),
        ("opaque", &opaque, libpng_oracle::COLOR_RGB),
    ];
    let dims = Dimensions::new(w, h).unwrap();
    for (name, src, expected_type) in cases {
        let mut reduced = Vec::new();
        PngEncoder::new()
            .with_compression(Level::Best)
            .with_auto_reduce(true)
            .encode_image(ImageRef::<Rgba8>::new(src, dims).unwrap(), &mut reduced)
            .expect("encode");
        assert_eq!(
            libpng_oracle::decode(&reduced).color_type,
            expected_type,
            "{name}: reduced to the expected colour type"
        );
        // Lossless: resolving the reduced image back to RGBA equals the source.
        let (_, _, rgba) = libpng_oracle::decode_rgba8(&reduced);
        assert_eq!(&rgba, src, "{name}: reduction is lossless");

        // Greyscale reduction (3 varying channels -> 1) is the robust size win; palette and
        // alpha-drop can merely tie an already-tiny RGBA stream once DEFLATE has exploited the
        // redundancy, so only assert a strict win for the greyscale case.
        if name == "gray" {
            let mut full = Vec::new();
            PngEncoder::new()
                .with_compression(Level::Best)
                .encode_image(ImageRef::<Rgba8>::new(src, dims).unwrap(), &mut full)
                .expect("encode");
            assert!(
                reduced.len() < full.len(),
                "gray: reduced {} should beat full {}",
                reduced.len(),
                full.len()
            );
        }
    }
}

#[test]
fn brute_force_filtering_round_trips_and_is_competitive() {
    let (w, h) = (48u32, 48u32);
    let src = rgb_pattern(w, h);
    let dims = Dimensions::new(w, h).unwrap();
    let encode = |filter| {
        let mut png = Vec::new();
        PngEncoder::new()
            .with_compression(Level::Best)
            .with_auto_reduce(false)
            .with_filter(filter)
            .encode_image(ImageRef::<Rgb8>::new(&src, dims).unwrap(), &mut png)
            .expect("encode");
        png
    };
    let brute = encode(FilterStrategy::BruteForce);
    assert_eq!(
        libpng_oracle::decode(&brute).pixels,
        src,
        "brute-force round-trip"
    );
    // Brute force tries MinSumAbs among its candidates, so it never loses to it.
    assert!(brute.len() <= encode(FilterStrategy::MinSumAbs).len());
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
