//! VP8L image-data encoder.
//!
//! Produces a conformant, bit-exact-lossless VP8L bitstream. The encoder applies the forward
//! transforms (subtract-green, then the spatial predictor, then the color transform), emitting each
//! transform and its sub-resolution data, and then codes the residual image as literal ARGB pixels
//! under a single prefix-code group. The remaining density features (color indexing, LZ77 backward
//! references, the color cache, and multi-group entropy images) layer on in later commits of the
//! issue-#22 series.
//!
//! Compression *quality* (optimal transform/mode choice, LZ77 parsing, entropy clustering) is
//! deferred to issue #31; this code only needs to emit a valid stream that round-trips. Each
//! channel's prefix code is built from the true histogram via
//! [`build_length_limited_lengths`](crate::vp8l::prefix::build_length_limited_lengths) and written
//! with the normal code-length code. Single-symbol codes (e.g. a solid color, or the unused distance
//! code) consume no bits, exactly as the decoder expects.

use gamut_core::{Dimensions, Error, Result};

use crate::vp8l::bit_io::BitWriter;
use crate::vp8l::header::Vp8lHeader;
use crate::vp8l::prefix::{
    MAX_CODE_LENGTH, NUM_DISTANCE_CODES, NUM_LITERAL_CODES, PrefixEncoder,
    build_length_limited_lengths, green_alphabet_size, write_normal_prefix_code,
};
use crate::vp8l::transform::{
    COLOR_TRANSFORM, PREDICTOR_TRANSFORM, SUBTRACT_GREEN_TRANSFORM, alpha, blue, forward_color,
    forward_predictor, forward_subtract_green, green, red,
};

/// Block-size exponent for the predictor/color sub-images (16×16 blocks). Optimal block sizing is a
/// density tuning knob deferred to issue #31.
const TRANSFORM_SIZE_BITS: u8 = 4;

/// Encodes an ARGB image (scan order, `0xAARRGGBB`) to a VP8L bitstream body — the bytes that go
/// inside the `VP8L` chunk, signature byte included.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if `argb.len()` does not equal `width * height`, or the
/// dimensions are out of the VP8L range.
pub fn encode(argb: &[u32], dims: Dimensions) -> Result<Vec<u8>> {
    let expected = (dims.width as usize).checked_mul(dims.height as usize);
    if expected != Some(argb.len()) {
        return Err(Error::InvalidInput(
            "VP8L: pixel buffer does not match dimensions",
        ));
    }
    let alpha_is_used = argb.iter().any(|&p| alpha(p) != 0xff);
    let header = Vp8lHeader::from_dimensions(dims, alpha_is_used)?;

    let mut w = BitWriter::new();
    header.write(&mut w);

    let mut pixels = argb.to_vec();
    let (width, height) = (dims.width, dims.height);

    // Apply the forward transforms in read order; the decoder inverts them last-first.
    // 1. Subtract-green (no transform data).
    forward_subtract_green(&mut pixels);
    write_transform_tag(&mut w, SUBTRACT_GREEN_TRANSFORM);

    // 2. Spatial predictor.
    let (residual, predictor_sub) = forward_predictor(&pixels, width, height, TRANSFORM_SIZE_BITS);
    pixels = residual;
    write_transform_tag(&mut w, PREDICTOR_TRANSFORM);
    w.write_bits(u32::from(TRANSFORM_SIZE_BITS - 2), 3);
    write_image(&mut w, &predictor_sub, false);

    // 3. Color transform.
    let color_sub = forward_color(&mut pixels, width, height, TRANSFORM_SIZE_BITS);
    write_transform_tag(&mut w, COLOR_TRANSFORM);
    w.write_bits(u32::from(TRANSFORM_SIZE_BITS - 2), 3);
    write_image(&mut w, &color_sub, false);

    w.write_bits(0, 1); // End of transforms.
    write_image(&mut w, &pixels, true);
    Ok(w.finish())
}

/// Writes a "transform present" bit followed by the 2-bit transform type.
fn write_transform_tag(w: &mut BitWriter, transform_type: u8) {
    w.write_bits(1, 1);
    w.write_bits(u32::from(transform_type), 2);
}

/// Writes one image (any role) as literal pixels under a single prefix-code group. `allow_meta`
/// matches the decoder: the top-level image writes the meta-prefix bit (single group), sub-images
/// do not.
fn write_image(w: &mut BitWriter, pixels: &[u32], allow_meta: bool) {
    w.write_bits(0, 1); // No color cache.
    if allow_meta {
        w.write_bits(0, 1); // Single meta prefix code.
    }

    let mut green_hist = vec![0u32; green_alphabet_size(0)];
    let mut red_hist = vec![0u32; NUM_LITERAL_CODES];
    let mut blue_hist = vec![0u32; NUM_LITERAL_CODES];
    let mut alpha_hist = vec![0u32; NUM_LITERAL_CODES];
    let mut distance_hist = vec![0u32; NUM_DISTANCE_CODES];
    for &p in pixels {
        green_hist[green(p) as usize] += 1;
        red_hist[red(p) as usize] += 1;
        blue_hist[blue(p) as usize] += 1;
        alpha_hist[alpha(p) as usize] += 1;
    }
    // No backward references: emit a valid single-symbol (symbol 0) distance code.
    distance_hist[0] = 1;

    let green_code = build_code(&green_hist);
    let red_code = build_code(&red_hist);
    let blue_code = build_code(&blue_hist);
    let alpha_code = build_code(&alpha_hist);
    let distance_code = build_code(&distance_hist);

    write_normal_prefix_code(w, green_code.lengths());
    write_normal_prefix_code(w, red_code.lengths());
    write_normal_prefix_code(w, blue_code.lengths());
    write_normal_prefix_code(w, alpha_code.lengths());
    write_normal_prefix_code(w, distance_code.lengths());

    for &p in pixels {
        green_code.write_symbol(w, green(p) as usize);
        red_code.write_symbol(w, red(p) as usize);
        blue_code.write_symbol(w, blue(p) as usize);
        alpha_code.write_symbol(w, alpha(p) as usize);
    }
}

/// Builds a [`PrefixEncoder`] from a histogram, forcing a valid single-symbol-0 code if the
/// histogram is empty (so an unused alphabet still emits a complete code).
fn build_code(histogram: &[u32]) -> PrefixEncoder {
    let mut lengths = build_length_limited_lengths(histogram, MAX_CODE_LENGTH as u8);
    // An all-zero histogram yields an empty code; code symbol 0 at length 1 so the tree is valid.
    if !lengths.is_empty() && lengths.iter().all(|&l| l == 0) {
        lengths[0] = 1;
    }
    PrefixEncoder::from_lengths(&lengths)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vp8l::decoder::decode;
    use crate::vp8l::transform::make_argb;

    fn round_trip(argb: &[u32], width: u32, height: u32) {
        let dims = Dimensions { width, height };
        let bitstream = encode(argb, dims).expect("encode");
        let (decoded_dims, pixels) = decode(&bitstream).expect("decode");
        assert_eq!(decoded_dims, dims);
        assert_eq!(pixels, argb, "round-trip mismatch at {width}x{height}");
    }

    #[test]
    fn round_trips_single_pixel() {
        round_trip(&[make_argb(0xff, 0x12, 0x34, 0x56)], 1, 1);
    }

    #[test]
    fn round_trips_gradient() {
        let (w, h) = (8u32, 5u32);
        let img: Vec<u32> = (0..w * h)
            .map(|i| make_argb(0xff, (i * 3) as u8, (i * 7) as u8, i as u8))
            .collect();
        round_trip(&img, w, h);
    }

    #[test]
    fn round_trips_solid_color() {
        let img = vec![make_argb(0xff, 9, 9, 9); 17 * 9];
        round_trip(&img, 17, 9);
    }

    #[test]
    fn preserves_non_opaque_alpha() {
        let img: Vec<u32> = (0..16)
            .map(|i| make_argb((i * 16) as u8, i as u8, (255 - i) as u8, 0))
            .collect();
        round_trip(&img, 4, 4);
        // The header's alpha hint should be set when any pixel is non-opaque.
        let bitstream = encode(
            &img,
            Dimensions {
                width: 4,
                height: 4,
            },
        )
        .unwrap();
        let mut r = crate::vp8l::bit_io::BitReader::new(&bitstream);
        let header = Vp8lHeader::read(&mut r).unwrap();
        assert!(header.alpha_is_used);
    }

    #[test]
    fn rejects_dimension_mismatch() {
        assert!(matches!(
            encode(
                &[0, 0, 0],
                Dimensions {
                    width: 2,
                    height: 2
                }
            ),
            Err(Error::InvalidInput(_))
        ));
    }
}
