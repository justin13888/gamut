//! The decoder rejects inconsistent metadata and honours the photometric interpretation, exercising
//! branches the well-formed oracle round-trips never reach (P10 / #110).
//!
//! These patch a single tag of an otherwise-valid file **in place** (preserving the strip pixel
//! data the decoder reads) so a mutated validation either decodes a wrong-but-`Ok` image, panics, or
//! flips the colours — all distinguishable from the correct clean rejection.

use gamut_core::{Bilevel, DecodeImage, Dimensions, EncodeImage, Gray8, ImageBuf, ImageRef, Rgb8};
use gamut_tiff::{Compression, TiffDecoder, TiffEncoder, tags};

fn valid_rgb(w: u32, h: u32) -> Vec<u8> {
    let rgb: Vec<u8> = (0..w * h * 3).map(|i| (i * 7) as u8).collect();
    let mut out = Vec::new();
    TiffEncoder::new()
        .encode_image(
            ImageRef::<Rgb8>::new(
                &rgb,
                Dimensions {
                    width: w,
                    height: h,
                },
            )
            .unwrap(),
            &mut out,
        )
        .unwrap();
    out
}

/// Overwrites the inline value of `tag` on the first IFD (little-endian classic TIFF), preserving
/// every strip/tile byte. Handles `SHORT` (type 3) and `LONG` (type 4) inline scalars.
fn patch_inline(tiff: &mut [u8], tag: u16, new: u32) {
    let ifd = u32::from_le_bytes(tiff[4..8].try_into().unwrap()) as usize;
    let n = u16::from_le_bytes(tiff[ifd..ifd + 2].try_into().unwrap()) as usize;
    for e in 0..n {
        let p = ifd + 2 + e * 12;
        if u16::from_le_bytes(tiff[p..p + 2].try_into().unwrap()) == tag {
            match u16::from_le_bytes(tiff[p + 2..p + 4].try_into().unwrap()) {
                3 => tiff[p + 8..p + 10].copy_from_slice(&(new as u16).to_le_bytes()),
                4 => tiff[p + 8..p + 12].copy_from_slice(&new.to_le_bytes()),
                ty => panic!("tag {tag} has non-inline type {ty}"),
            }
            return;
        }
    }
    panic!("tag {tag} not found");
}

fn errs(tiff: &[u8]) -> bool {
    // The correct decoder rejects these cleanly (Err, no panic). A mutated guard that lets the file
    // through then trips a later bounds/zero check and panics — which fails this test, so the mutant
    // is caught either way.
    TiffDecoder::new().decode_page(tiff, 0).is_err()
}

#[test]
fn rejects_zero_height() {
    // height 0 (width non-zero) must be rejected; both `width == 0` and `height == 0` are real,
    // so the guard cannot collapse to a single conjunction.
    let mut v = valid_rgb(8, 8);
    patch_inline(&mut v, tags::IMAGE_LENGTH, 0);
    assert!(errs(&v));
}

#[test]
fn rejects_inconsistent_sample_count() {
    // SamplesPerPixel says 1 but BitsPerSample still lists three entries: the `bits.len() != spp`
    // check must fire. Also flip the photometric to BlackIsZero so that, if a mutated `&&` lets the
    // file through, it forms a *valid* (1, 8, gray) layout and decodes a wrong-but-`Ok` image — which
    // the rejection check above must instead refuse.
    let mut v = valid_rgb(8, 8);
    patch_inline(&mut v, tags::SAMPLES_PER_PIXEL, 1);
    patch_inline(&mut v, tags::PHOTOMETRIC_INTERPRETATION, 1); // -> BlackIsZero (valid for 1 sample)
    assert!(TiffDecoder::new().decode_page(&v, 0).is_err());
}

#[test]
fn rejects_zero_tile_dimension() {
    let rgb: Vec<u8> = (0..32 * 32 * 3).map(|i| (i * 7) as u8).collect();
    let mut v = Vec::new();
    TiffEncoder::new()
        .with_tiling(16, 16)
        .encode_image(
            ImageRef::<Rgb8>::new(
                &rgb,
                Dimensions {
                    width: 32,
                    height: 32,
                },
            )
            .unwrap(),
            &mut v,
        )
        .unwrap();
    patch_inline(&mut v, tags::TILE_WIDTH, 0);
    assert!(errs(&v));
}

#[test]
fn honours_white_is_zero_photometric() {
    // gamut writes bilevel as BlackIsZero (1). Flip PhotometricInterpretation to WhiteIsZero (0) in
    // place; the same stored bits must now decode to the inverted image — pinning the `bit == 0` test
    // on the WhiteIsZero branch the encoder itself never produces.
    let (w, h) = (13u32, 9u32);
    let src: Vec<u8> = (0..w * h)
        .map(|i| if i % 3 == 0 { 255 } else { 0 })
        .collect();
    let mut tiff = Vec::new();
    TiffEncoder::new()
        .with_compression(Compression::None)
        .encode_image(
            ImageRef::<Bilevel>::new(
                &src,
                Dimensions {
                    width: w,
                    height: h,
                },
            )
            .unwrap(),
            &mut tiff,
        )
        .unwrap();
    patch_inline(&mut tiff, tags::PHOTOMETRIC_INTERPRETATION, 0); // BlackIsZero -> WhiteIsZero
    let got: ImageBuf<Gray8> = TiffDecoder::new().decode_image(&tiff).expect("decode");
    let inverted: Vec<u8> = src.iter().map(|&v| 255 - v).collect();
    assert_eq!(got.as_samples(), inverted.as_slice());
}

/// Increments the element count of an out-of-line array tag by 1, leaving the (now one-short) array
/// data in place. The decoder reads one extra, ignored, element — so the only effect is that the
/// tag's count no longer equals the strip/tile count, which the consistency check must reject.
fn bump_count(tiff: &mut [u8], tag: u16) {
    let ifd = u32::from_le_bytes(tiff[4..8].try_into().unwrap()) as usize;
    let n = u16::from_le_bytes(tiff[ifd..ifd + 2].try_into().unwrap()) as usize;
    for e in 0..n {
        let p = ifd + 2 + e * 12;
        if u16::from_le_bytes(tiff[p..p + 2].try_into().unwrap()) == tag {
            let c = u32::from_le_bytes(tiff[p + 4..p + 8].try_into().unwrap());
            tiff[p + 4..p + 8].copy_from_slice(&(c + 1).to_le_bytes());
            return;
        }
    }
    panic!("tag {tag} not found");
}

#[test]
fn rejects_strip_count_mismatch() {
    // A tall image is split into several strips. One extra (ignored) StripOffsets entry makes
    // offsets.len() != strips while StripByteCounts.len() still equals it — so the guard must fire on
    // the offset disjunct alone (a mutated `&&` would decode the image fine and accept it).
    let mut v = valid_rgb(200, 100);
    bump_count(&mut v, tags::STRIP_OFFSETS);
    assert!(TiffDecoder::new().decode_page(&v, 0).is_err());
}

#[test]
fn rejects_tile_count_mismatch() {
    let rgb: Vec<u8> = (0..32 * 32 * 3).map(|i| (i * 7) as u8).collect();
    let mut v = Vec::new();
    TiffEncoder::new()
        .with_tiling(16, 16)
        .encode_image(
            ImageRef::<Rgb8>::new(
                &rgb,
                Dimensions {
                    width: 32,
                    height: 32,
                },
            )
            .unwrap(),
            &mut v,
        )
        .unwrap();
    bump_count(&mut v, tags::TILE_OFFSETS);
    assert!(TiffDecoder::new().decode_page(&v, 0).is_err());
}
