//! 16-bit samples (§19): native decode over strips and tiles in both byte orders, the
//! widening/narrowing presentation policy, and bidirectional libtiff parity.

use gamut_core::{
    DecodeImage, Dimensions, EncodeImage, Gray8, Gray16, ImageBuf, ImageRef, Rgb8, Rgb16, Rgba16,
};
use gamut_tiff::{ByteOrder, Compression, Predictor, TiffDecoder, TiffEncoder, read, tags};
use libtiff_oracle::Compression as OC;

/// Sizes include 1x1, a size smaller than one tile, and 17x13 — deliberately not a multiple of 16,
/// so every tiled case exercises the cropped right and bottom edge tiles.
const SIZES: &[(u32, u32)] = &[(1, 1), (3, 7), (17, 13), (40, 24)];

/// A 16-bit pattern whose low byte varies independently of its high byte.
///
/// That independence is the point: a value like `i * 257` has identical bytes, so a byte-swap bug
/// would round-trip it unchanged and every endianness test built on it would pass vacuously.
fn pattern16(count: usize, salt: u32) -> Vec<u16> {
    (0..count)
        .map(|i| {
            let i = i as u32;
            ((i.wrapping_mul(2711).wrapping_add(salt.wrapping_mul(7919))) % 65536) as u16
        })
        .collect()
}

fn gray16_pattern(w: u32, h: u32) -> Vec<u16> {
    pattern16((w * h) as usize, 1)
}

fn rgb16_pattern(w: u32, h: u32) -> Vec<u16> {
    pattern16((w * h * 3) as usize, 2)
}

fn dims(w: u32, h: u32) -> Dimensions {
    Dimensions {
        width: w,
        height: h,
    }
}

/// Every gamut compression that can carry 16-bit samples, paired with the predictors it allows.
/// CCITT is bilevel-only and so is absent by construction.
const GAMUT_MODES: &[(Compression, Predictor)] = &[
    (Compression::None, Predictor::None),
    (Compression::PackBits, Predictor::None),
    (Compression::Lzw, Predictor::None),
    (Compression::Lzw, Predictor::HorizontalDifferencing),
    (Compression::Deflate, Predictor::None),
    (Compression::Deflate, Predictor::HorizontalDifferencing),
];

fn encoder(order: ByteOrder, compression: Compression, predictor: Predictor) -> TiffEncoder {
    TiffEncoder::new()
        .with_byte_order(order)
        .with_compression(compression)
        .with_predictor(predictor)
}

#[test]
fn gray16_roundtrips_in_gamut() {
    for &(w, h) in SIZES {
        let src = gray16_pattern(w, h);
        for order in [ByteOrder::LittleEndian, ByteOrder::BigEndian] {
            for &(compression, predictor) in GAMUT_MODES {
                let tiff = encoder(order, compression, predictor)
                    .encode_to_vec(ImageRef::<Gray16>::new(&src, dims(w, h)).unwrap())
                    .expect("encode");
                let got: ImageBuf<Gray16> = TiffDecoder::new().decode_image(&tiff).expect("decode");
                assert_eq!(got.dimensions(), dims(w, h));
                assert_eq!(
                    got.as_samples(),
                    src.as_slice(),
                    "gray16 {compression:?} {predictor:?} {order:?} at {w}x{h}"
                );
            }
        }
    }
}

#[test]
fn rgb16_and_rgba16_roundtrip_in_gamut() {
    for &(w, h) in SIZES {
        let rgb = rgb16_pattern(w, h);
        let rgba = pattern16((w * h * 4) as usize, 3);
        for order in [ByteOrder::LittleEndian, ByteOrder::BigEndian] {
            for &(compression, predictor) in GAMUT_MODES {
                let enc = encoder(order, compression, predictor);
                let tiff = enc
                    .encode_to_vec(ImageRef::<Rgb16>::new(&rgb, dims(w, h)).unwrap())
                    .expect("encode");
                let got: ImageBuf<Rgb16> = TiffDecoder::new().decode_image(&tiff).expect("decode");
                assert_eq!(
                    got.as_samples(),
                    rgb.as_slice(),
                    "rgb16 {compression:?} {predictor:?} {order:?} at {w}x{h}"
                );

                let tiff = enc
                    .encode_to_vec(ImageRef::<Rgba16>::new(&rgba, dims(w, h)).unwrap())
                    .expect("encode");
                let got: ImageBuf<Rgba16> = TiffDecoder::new().decode_image(&tiff).expect("decode");
                assert_eq!(
                    got.as_samples(),
                    rgba.as_slice(),
                    "rgba16 {compression:?} {predictor:?} {order:?} at {w}x{h}"
                );
            }
        }
    }
}

#[test]
fn rgb16_roundtrips_in_gamut_as_tiles() {
    // Tiles are predicted independently and the encoder's tiled predictor is Deflate-only, so this
    // covers both the padded-edge blit and the per-tile predictor at 16 bits.
    for &(w, h) in SIZES {
        let src = rgb16_pattern(w, h);
        for order in [ByteOrder::LittleEndian, ByteOrder::BigEndian] {
            for (compression, predictor) in [
                (Compression::None, Predictor::None),
                (Compression::Lzw, Predictor::None),
                (Compression::Deflate, Predictor::HorizontalDifferencing),
            ] {
                let tiff = encoder(order, compression, predictor)
                    .with_tiling(16, 16)
                    .encode_to_vec(ImageRef::<Rgb16>::new(&src, dims(w, h)).unwrap())
                    .expect("encode");
                let got: ImageBuf<Rgb16> = TiffDecoder::new().decode_image(&tiff).expect("decode");
                assert_eq!(
                    got.as_samples(),
                    src.as_slice(),
                    "rgb16 tiled {compression:?} {predictor:?} {order:?} at {w}x{h}"
                );
            }
        }
    }
}

#[test]
fn gamut_sixteen_bit_is_decoded_by_libtiff() {
    // The other direction, and the one that actually pins the encoder's byte order: a gamut-only
    // round-trip would survive a swapped pack/unpack pair, since it writes and reads with the same
    // code. libtiff reading an `MM` file gamut wrote cannot.
    for &(w, h) in SIZES {
        let gray = gray16_pattern(w, h);
        let rgb = rgb16_pattern(w, h);
        for order in [ByteOrder::LittleEndian, ByteOrder::BigEndian] {
            for &(compression, predictor) in GAMUT_MODES {
                let enc = encoder(order, compression, predictor);

                let tiff = enc
                    .encode_to_vec(ImageRef::<Gray16>::new(&gray, dims(w, h)).unwrap())
                    .expect("encode");
                let oracle = libtiff_oracle::decode_tiff16(&tiff).expect("libtiff decode");
                assert_eq!((oracle.width, oracle.height), (w, h));
                assert_eq!(
                    oracle.samples, gray,
                    "gray16 {compression:?} {predictor:?} {order:?} at {w}x{h}"
                );

                let tiff = enc
                    .encode_to_vec(ImageRef::<Rgb16>::new(&rgb, dims(w, h)).unwrap())
                    .expect("encode");
                let oracle = libtiff_oracle::decode_tiff16(&tiff).expect("libtiff decode");
                assert_eq!(
                    oracle.samples, rgb,
                    "rgb16 {compression:?} {predictor:?} {order:?} at {w}x{h}"
                );
            }
        }
    }
}

#[test]
fn gamut_tiled_sixteen_bit_is_decoded_by_libtiff() {
    for &(w, h) in SIZES {
        let src = rgb16_pattern(w, h);
        for order in [ByteOrder::LittleEndian, ByteOrder::BigEndian] {
            for (compression, predictor) in [
                (Compression::Lzw, Predictor::None),
                (Compression::Deflate, Predictor::HorizontalDifferencing),
            ] {
                let tiff = encoder(order, compression, predictor)
                    .with_tiling(16, 16)
                    .encode_to_vec(ImageRef::<Rgb16>::new(&src, dims(w, h)).unwrap())
                    .expect("encode");
                let oracle = libtiff_oracle::decode_tiff16(&tiff).expect("libtiff decode");
                assert_eq!(
                    oracle.samples, src,
                    "rgb16 tiled {compression:?} {predictor:?} {order:?} at {w}x{h}"
                );
            }
        }
    }
}

#[test]
fn sixteen_bit_encode_declares_its_depth_and_alpha() {
    let src = pattern16(4 * 4 * 4, 5);
    let tiff = TiffEncoder::new()
        .encode_to_vec(ImageRef::<Rgba16>::new(&src, dims(4, 4)).unwrap())
        .expect("encode");
    let ifd = &read(&tiff).expect("read").ifds[0];
    assert_eq!(
        ifd.get_u32_vec(tags::BITS_PER_SAMPLE),
        Some(vec![16, 16, 16, 16])
    );
    // Unassociated alpha, matching the 8-bit RGBA impl.
    assert_eq!(ifd.get_u32_vec(tags::EXTRA_SAMPLES), Some(vec![2]));
}

#[test]
fn sixteen_bit_rejects_ccitt_which_is_bilevel_only() {
    let src = gray16_pattern(8, 8);
    let got = TiffEncoder::new()
        .with_compression(Compression::CcittGroup4Fax)
        .encode_to_vec(ImageRef::<Gray16>::new(&src, dims(8, 8)).unwrap());
    assert!(got.is_err(), "CCITT cannot carry 16-bit samples");
}

#[test]
fn libtiff_gray16_is_decoded_by_gamut() {
    for &(w, h) in SIZES {
        let src = gray16_pattern(w, h);
        for big_endian in [false, true] {
            for (compression, predictor) in [
                (OC::None, 1u16),
                (OC::PackBits, 1),
                (OC::Lzw, 1),
                (OC::Lzw, 2),
                (OC::Deflate, 2),
            ] {
                let tiff =
                    libtiff_oracle::encode_gray16(&src, w, h, compression, predictor, big_endian)
                        .expect("libtiff encode");
                let got: ImageBuf<Gray16> = TiffDecoder::new()
                    .decode_image(&tiff)
                    .expect("gamut decode");
                assert_eq!(got.dimensions(), dims(w, h));
                assert_eq!(
                    got.as_samples(),
                    src.as_slice(),
                    "gray16 {compression:?} p{predictor} be={big_endian} at {w}x{h}"
                );
            }
        }
    }
}

#[test]
fn libtiff_rgb16_is_decoded_by_gamut() {
    for &(w, h) in SIZES {
        let src = rgb16_pattern(w, h);
        for big_endian in [false, true] {
            for (compression, predictor) in [
                (OC::None, 1u16),
                (OC::PackBits, 1),
                (OC::Lzw, 1),
                (OC::Lzw, 2),
                (OC::Deflate, 2),
            ] {
                let tiff =
                    libtiff_oracle::encode_rgb16(&src, w, h, compression, predictor, big_endian)
                        .expect("libtiff encode");
                let got: ImageBuf<Rgb16> = TiffDecoder::new()
                    .decode_image(&tiff)
                    .expect("gamut decode");
                assert_eq!(got.dimensions(), dims(w, h));
                assert_eq!(
                    got.as_samples(),
                    src.as_slice(),
                    "rgb16 {compression:?} p{predictor} be={big_endian} at {w}x{h}"
                );
            }
        }
    }
}

#[test]
fn libtiff_rgb16_tiled_is_decoded_by_gamut() {
    // Tiles are predicted independently, and every non-multiple-of-16 size leaves padded right and
    // bottom edges to crop — both are places a widened byte stride can go wrong silently.
    for &(w, h) in SIZES {
        let src = rgb16_pattern(w, h);
        for (compression, predictor) in [(OC::None, 1u16), (OC::Lzw, 1), (OC::Deflate, 2)] {
            let tiff =
                libtiff_oracle::encode_rgb16_tiled(&src, w, h, 16, 16, compression, predictor)
                    .expect("libtiff encode");
            let got: ImageBuf<Rgb16> = TiffDecoder::new()
                .decode_image(&tiff)
                .expect("gamut decode");
            assert_eq!(got.dimensions(), dims(w, h));
            assert_eq!(
                got.as_samples(),
                src.as_slice(),
                "rgb16 tiled {compression:?} p{predictor} at {w}x{h}"
            );
        }
    }
}

#[test]
fn big_endian_sixteen_bit_is_not_byte_swapped() {
    // The endianness pin. A gamut-only round-trip is symmetric — it writes and reads with the same
    // code, so a swapped load paired with a swapped store cancels out and the test still passes.
    // Only an independently produced `MM` file catches it, and the asserted value is exact.
    let src: Vec<u16> = vec![0x0102, 0xFFFE, 0x00FF, 0xFF00];
    let tiff =
        libtiff_oracle::encode_gray16(&src, 4, 1, OC::None, 1, true).expect("libtiff encode");
    assert_eq!(
        &tiff[..2],
        b"MM",
        "the oracle must have written a big-endian file"
    );
    let got: ImageBuf<Gray16> = TiffDecoder::new()
        .decode_image(&tiff)
        .expect("gamut decode");
    assert_eq!(got.as_samples(), src.as_slice());
}

#[test]
fn eight_bit_widens_to_sixteen_by_257() {
    // x257 is exact at both ends (0 -> 0, 255 -> 65535) and is the inverse of the >> 8 narrowing
    // below. Asserting the values rather than a round-trip is what distinguishes it from x256.
    let src: Vec<u8> = vec![0, 1, 128, 255];
    let mut tiff = Vec::new();
    TiffEncoder::new()
        .encode_image(ImageRef::<Gray8>::new(&src, dims(4, 1)).unwrap(), &mut tiff)
        .expect("encode");
    let got: ImageBuf<Gray16> = TiffDecoder::new().decode_image(&tiff).expect("decode");
    assert_eq!(got.as_samples(), &[0u16, 257, 32896, 65535]);
}

#[test]
fn sixteen_bit_narrows_to_eight_by_truncation() {
    // Truncation, not rounding: 0x01FF must become 0x01, never 0x02. Documented as lossy, and the
    // exact inverse of the widening above.
    let src: Vec<u16> = vec![0x0000, 0x00FF, 0x0100, 0x01FF, 0xFFFF];
    let tiff = libtiff_oracle::encode_gray16(&src, 5, 1, OC::None, 1, false).expect("libtiff");
    let got: ImageBuf<Gray8> = TiffDecoder::new().decode_image(&tiff).expect("decode");
    assert_eq!(got.as_samples(), &[0x00u8, 0x00, 0x01, 0x01, 0xFF]);
}

#[test]
fn sixteen_bit_rgb_narrows_through_the_eight_bit_surface() {
    // The behaviour change this policy introduces: before 16-bit support these calls returned
    // `Unsupported`. `decode_page` and `DecodeImage<Rgb8>` must now both narrow.
    let (w, h) = (17u32, 13u32);
    let src = rgb16_pattern(w, h);
    let tiff = libtiff_oracle::encode_rgb16(&src, w, h, OC::Lzw, 1, false).expect("libtiff");
    let expected: Vec<u8> = src.iter().map(|&v| (v >> 8) as u8).collect();

    let got: ImageBuf<Rgb8> = TiffDecoder::new().decode_image(&tiff).expect("decode");
    assert_eq!(got.as_samples(), expected.as_slice());
    let page = TiffDecoder::new()
        .decode_page(&tiff, 0)
        .expect("decode_page");
    assert_eq!(page.as_samples(), expected.as_slice());
}

#[test]
fn gray16_replicates_and_gains_opaque_alpha() {
    // Channel widening is orthogonal to sample widening: a single-sample 16-bit page must present
    // as RGB16 by replication and as RGBA16 with a full-range opaque alpha (not 255).
    let src: Vec<u16> = vec![0x1234, 0xABCD];
    let tiff = libtiff_oracle::encode_gray16(&src, 2, 1, OC::None, 1, false).expect("libtiff");

    let rgb: ImageBuf<Rgb16> = TiffDecoder::new().decode_image(&tiff).expect("decode");
    assert_eq!(
        rgb.as_samples(),
        &[0x1234, 0x1234, 0x1234, 0xABCD, 0xABCD, 0xABCD]
    );

    let rgba: ImageBuf<Rgba16> = TiffDecoder::new().decode_image(&tiff).expect("decode");
    assert_eq!(
        rgba.as_samples(),
        &[
            0x1234, 0x1234, 0x1234, 0xFFFF, 0xABCD, 0xABCD, 0xABCD, 0xFFFF
        ]
    );
}

#[test]
fn gray16_rejects_a_multi_sample_page() {
    let src = rgb16_pattern(4, 4);
    let tiff = libtiff_oracle::encode_rgb16(&src, 4, 4, OC::None, 1, false).expect("libtiff");
    let got: Result<ImageBuf<Gray16>, _> = TiffDecoder::new().decode_image(&tiff);
    assert!(got.is_err(), "an RGB page cannot be presented as Gray16");
}

#[test]
fn white_is_zero_inverts_at_sixteen_bits() {
    // The 16-bit sibling of the 8-bit WhiteIsZero test. libtiff writes MINISWHITE faithfully, so
    // the same stored samples must come back inverted against the full 16-bit range — the only
    // thing pinning `u16::MAX - v` rather than a stray `255 - v`.
    let src: Vec<u16> = vec![0x0000, 0x1234, 0xFFFF, 0x00FF];
    let tiff = libtiff_oracle::encode_gray16_miniswhite(&src, 4, 1, OC::None).expect("libtiff");
    let got: ImageBuf<Gray16> = TiffDecoder::new().decode_image(&tiff).expect("decode");
    let inverted: Vec<u16> = src.iter().map(|&v| u16::MAX - v).collect();
    assert_eq!(got.as_samples(), inverted.as_slice());
}
