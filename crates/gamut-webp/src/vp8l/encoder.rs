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
use crate::vp8l::color_cache::ColorCache;
use crate::vp8l::header::Vp8lHeader;
use crate::vp8l::lz77::{BackwardRefs, pixel_distance_to_code, value_to_prefix};
use crate::vp8l::prefix::{
    MAX_CODE_LENGTH, NUM_DISTANCE_CODES, NUM_LENGTH_CODES, NUM_LITERAL_CODES, PrefixEncoder,
    build_length_limited_lengths, green_alphabet_size, write_normal_prefix_code,
};
use crate::vp8l::transform::{
    COLOR_INDEXING_TRANSFORM, COLOR_TRANSFORM, PREDICTOR_TRANSFORM, SUBTRACT_GREEN_TRANSFORM,
    alpha, blue, forward_color, forward_color_indexing, forward_predictor, forward_subtract_green,
    green, red, subtract_pixels,
};

/// Block-size exponent for the predictor/color sub-images (16×16 blocks). Optimal block sizing is a
/// density tuning knob deferred to issue #31.
const TRANSFORM_SIZE_BITS: u8 = 4;

/// Maximum number of distinct colors for the color-indexing (palette) transform (RFC 9649 §4.4).
const MAX_PALETTE_SIZE: usize = 256;

/// First green-alphabet symbol denoting an LZ77 length code.
const LENGTH_CODE_BASE: usize = NUM_LITERAL_CODES;
/// First green-alphabet symbol denoting a color-cache index.
const CACHE_CODE_BASE: usize = NUM_LITERAL_CODES + NUM_LENGTH_CODES;

/// One coding decision for the entropy-coded pixel stream.
enum Token {
    /// A literal ARGB pixel (green, red, blue, alpha symbols).
    Literal(u32),
    /// An LZ77 backward reference: copy `len` pixels from `dist` pixels back.
    Copy { len: u32, dist: u32 },
    /// A color-cache hit at the given slot.
    CacheIndex(u16),
}

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

    // Few-color images take the palette path; everything else takes the spatial-transform path.
    // Choosing the densest path for a given image is a tuning concern deferred to issue #31.
    match build_palette(argb) {
        Some(palette) => encode_palette(&mut w, argb, dims, &palette),
        None => encode_spatial(&mut w, argb, dims),
    }
    Ok(w.finish())
}

/// Encodes via the spatial transforms (subtract-green, predictor, color) applied in read order; the
/// decoder inverts them last-first.
fn encode_spatial(w: &mut BitWriter, argb: &[u32], dims: Dimensions) {
    let (width, height) = (dims.width, dims.height);
    let mut pixels = argb.to_vec();

    forward_subtract_green(&mut pixels);
    write_transform_tag(w, SUBTRACT_GREEN_TRANSFORM);

    let (residual, predictor_sub) = forward_predictor(&pixels, width, height, TRANSFORM_SIZE_BITS);
    pixels = residual;
    write_transform_tag(w, PREDICTOR_TRANSFORM);
    w.write_bits(u32::from(TRANSFORM_SIZE_BITS - 2), 3);
    write_sub_image(w, &predictor_sub);

    let color_sub = forward_color(&mut pixels, width, height, TRANSFORM_SIZE_BITS);
    write_transform_tag(w, COLOR_TRANSFORM);
    w.write_bits(u32::from(TRANSFORM_SIZE_BITS - 2), 3);
    write_sub_image(w, &color_sub);

    w.write_bits(0, 1); // End of transforms.
    write_main_image(w, &pixels, width);
}

/// Encodes via the color-indexing (palette) transform: the subtraction-coded palette followed by the
/// bundled index image (RFC 9649 §4.4).
fn encode_palette(w: &mut BitWriter, argb: &[u32], dims: Dimensions, palette: &[u32]) {
    write_transform_tag(w, COLOR_INDEXING_TRANSFORM);
    w.write_bits((palette.len() - 1) as u32, 8);

    // The palette is stored as a height-1 image, subtraction-coded onto the previous entry.
    let mut palette_image = vec![0u32; palette.len()];
    palette_image[0] = palette[0];
    for i in 1..palette.len() {
        palette_image[i] = subtract_pixels(palette[i], palette[i - 1]);
    }
    write_sub_image(w, &palette_image);

    let (bundled, bundled_width) = forward_color_indexing(argb, dims.width, dims.height, palette);
    w.write_bits(0, 1); // End of transforms.
    write_main_image(w, &bundled, bundled_width);
}

/// Collects the distinct colors in first-seen order, or `None` if there are more than
/// [`MAX_PALETTE_SIZE`] (in which case the palette transform does not apply). Sorting the palette for
/// denser subtraction coding is deferred to issue #31.
fn build_palette(pixels: &[u32]) -> Option<Vec<u32>> {
    use std::collections::HashSet;
    let mut seen = HashSet::new();
    let mut palette = Vec::new();
    for &p in pixels {
        if seen.insert(p) {
            if palette.len() == MAX_PALETTE_SIZE {
                return None;
            }
            palette.push(p);
        }
    }
    Some(palette)
}

/// Writes a "transform present" bit followed by the 2-bit transform type.
fn write_transform_tag(w: &mut BitWriter, transform_type: u8) {
    w.write_bits(1, 1);
    w.write_bits(u32::from(transform_type), 2);
}

/// A built prefix-code group ready to emit symbols with.
struct CodeGroup {
    green: PrefixEncoder,
    red: PrefixEncoder,
    blue: PrefixEncoder,
    alpha: PrefixEncoder,
    distance: PrefixEncoder,
}

impl CodeGroup {
    /// Writes the five code descriptions in bitstream order.
    fn write_descriptions(&self, w: &mut BitWriter) {
        write_normal_prefix_code(w, self.green.lengths());
        write_normal_prefix_code(w, self.red.lengths());
        write_normal_prefix_code(w, self.blue.lengths());
        write_normal_prefix_code(w, self.alpha.lengths());
        write_normal_prefix_code(w, self.distance.lengths());
    }

    /// Emits a literal pixel's four channel symbols.
    fn write_literal(&self, w: &mut BitWriter, p: u32) {
        self.green.write_symbol(w, green(p) as usize);
        self.red.write_symbol(w, red(p) as usize);
        self.blue.write_symbol(w, blue(p) as usize);
        self.alpha.write_symbol(w, alpha(p) as usize);
    }
}

/// Writes a sub-resolution image (predictor/color sub-images, palette) as literal pixels: no color
/// cache, no meta prefix codes (the decoder reads these with `allow_meta = false`).
fn write_sub_image(w: &mut BitWriter, pixels: &[u32]) {
    w.write_bits(0, 1); // No color cache.

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
    distance_hist[0] = 1; // No backward references; emit a valid single-symbol distance code.

    let codes = CodeGroup {
        green: build_code(&green_hist),
        red: build_code(&red_hist),
        blue: build_code(&blue_hist),
        alpha: build_code(&alpha_hist),
        distance: build_code(&distance_hist),
    };
    codes.write_descriptions(w);
    for &p in pixels {
        codes.write_literal(w, p);
    }
}

/// Writes the top-level image with a color cache and LZ77 backward references under a single
/// prefix-code group (the meta-prefix bit is written here).
fn write_main_image(w: &mut BitWriter, pixels: &[u32], width: u32) {
    let cache_bits = pick_cache_bits(pixels.len());
    let cache_size = if cache_bits > 0 {
        1usize << cache_bits
    } else {
        0
    };

    if cache_bits > 0 {
        w.write_bits(1, 1);
        w.write_bits(cache_bits, 4);
    } else {
        w.write_bits(0, 1);
    }
    w.write_bits(0, 1); // Single meta prefix code.

    let tokens = tokenize(pixels, cache_bits);

    let mut green_hist = vec![0u32; green_alphabet_size(cache_size)];
    let mut red_hist = vec![0u32; NUM_LITERAL_CODES];
    let mut blue_hist = vec![0u32; NUM_LITERAL_CODES];
    let mut alpha_hist = vec![0u32; NUM_LITERAL_CODES];
    let mut distance_hist = vec![0u32; NUM_DISTANCE_CODES];
    for token in &tokens {
        match *token {
            Token::Literal(p) => {
                green_hist[green(p) as usize] += 1;
                red_hist[red(p) as usize] += 1;
                blue_hist[blue(p) as usize] += 1;
                alpha_hist[alpha(p) as usize] += 1;
            }
            Token::Copy { len, dist } => {
                green_hist[LENGTH_CODE_BASE + value_to_prefix(len).0 as usize] += 1;
                let dist_code = pixel_distance_to_code(dist, width);
                distance_hist[value_to_prefix(dist_code).0 as usize] += 1;
            }
            Token::CacheIndex(idx) => green_hist[CACHE_CODE_BASE + idx as usize] += 1,
        }
    }

    let codes = CodeGroup {
        green: build_code(&green_hist),
        red: build_code(&red_hist),
        blue: build_code(&blue_hist),
        alpha: build_code(&alpha_hist),
        distance: build_code(&distance_hist),
    };
    codes.write_descriptions(w);

    for token in &tokens {
        match *token {
            Token::Literal(p) => codes.write_literal(w, p),
            Token::Copy { len, dist } => {
                let (len_code, len_bits, len_extra) = value_to_prefix(len);
                codes
                    .green
                    .write_symbol(w, LENGTH_CODE_BASE + len_code as usize);
                w.write_bits(len_extra, u32::from(len_bits));
                let dist_code = pixel_distance_to_code(dist, width);
                let (dist_sym, dist_bits, dist_extra) = value_to_prefix(dist_code);
                codes.distance.write_symbol(w, dist_sym as usize);
                w.write_bits(dist_extra, u32::from(dist_bits));
            }
            Token::CacheIndex(idx) => {
                codes.green.write_symbol(w, CACHE_CODE_BASE + idx as usize);
            }
        }
    }
}

/// Tokenizes `pixels` into literals, LZ77 copies, and color-cache hits, simulating the cache exactly
/// as the decoder will reconstruct it (every produced pixel is inserted, in stream order). Token
/// preference is copy > cache > literal — a simple deterministic policy; optimal parsing is deferred
/// to issue #31.
fn tokenize(pixels: &[u32], cache_bits: u32) -> Vec<Token> {
    let n = pixels.len();
    let mut tokens = Vec::new();
    let mut refs = BackwardRefs::new(n);
    let mut cache = if cache_bits > 0 {
        ColorCache::new(cache_bits).ok()
    } else {
        None
    };

    let mut i = 0;
    while i < n {
        if let Some((len, dist)) = refs.find(pixels, i) {
            tokens.push(Token::Copy { len, dist });
            let end = i + len as usize;
            while i < end {
                refs.insert(pixels, i);
                if let Some(c) = cache.as_mut() {
                    c.insert(pixels[i]);
                }
                i += 1;
            }
        } else {
            let pixel = pixels[i];
            let hit_slot = cache.as_ref().and_then(|c| {
                let slot = c.slot(pixel);
                (c.lookup(slot as u32) == pixel).then_some(slot)
            });
            match hit_slot {
                Some(slot) => tokens.push(Token::CacheIndex(slot as u16)),
                None => tokens.push(Token::Literal(pixel)),
            }
            if let Some(c) = cache.as_mut() {
                c.insert(pixel);
            }
            refs.insert(pixels, i);
            i += 1;
        }
    }
    tokens
}

/// Chooses a color-cache size for an image of `num_pixels` pixels: off for tiny images, otherwise
/// roughly `ceil(log2(num_pixels))` capped to 10. Optimal sizing is deferred to issue #31.
fn pick_cache_bits(num_pixels: usize) -> u32 {
    if num_pixels < 16 {
        0
    } else {
        (usize::BITS - (num_pixels - 1).leading_zeros()).clamp(1, 10)
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
    fn round_trips_many_colors_via_spatial_path() {
        // > 256 distinct colors forces the spatial-transform path (subtract-green/predictor/color).
        let (w, h) = (64u32, 48u32);
        let img: Vec<u32> = (0..(w * h))
            .map(|i| {
                make_argb(
                    0xff,
                    (i & 0xff) as u8,
                    ((i >> 8) & 0xff) as u8,
                    (i * 13) as u8,
                )
            })
            .collect();
        assert!(
            build_palette(&img).is_none(),
            "test image must exceed the palette limit"
        );
        round_trip(&img, w, h);
    }

    #[test]
    fn round_trips_repetitive_spatial_with_backward_refs() {
        // A >256-color tile repeated horizontally, so the spatial residual has LZ77 matches.
        let (tile_w, h) = (40u32, 12u32);
        let tile: Vec<u32> = (0..(tile_w * h))
            .map(|i| {
                make_argb(
                    0xff,
                    (i & 0xff) as u8,
                    ((i >> 8) & 0xff) as u8,
                    (i * 7) as u8,
                )
            })
            .collect();
        let w = tile_w * 2;
        let img: Vec<u32> = (0..(w * h))
            .map(|i| {
                let (x, y) = (i % w, i / w);
                tile[(y * tile_w + x % tile_w) as usize]
            })
            .collect();
        assert!(build_palette(&img).is_none());
        round_trip(&img, w, h);
    }

    #[test]
    fn round_trips_repetitive_palette_with_backward_refs() {
        // A repetitive few-color image: palette path with the cache + LZ77 on the bundled indices.
        let palette = [
            make_argb(0xff, 1, 2, 3),
            make_argb(0xff, 4, 5, 6),
            make_argb(0xff, 7, 8, 9),
        ];
        let (w, h) = (32u32, 8u32);
        let img: Vec<u32> = (0..(w * h)).map(|i| palette[(i % 3) as usize]).collect();
        round_trip(&img, w, h);
    }

    #[test]
    fn round_trips_horizontal_run() {
        // Long horizontal runs of one color exercise distance-1 (run-length) backward references.
        let (w, h) = (50u32, 4u32);
        let img: Vec<u32> = (0..(w * h))
            .map(|i| {
                if (i / w) % 2 == 0 {
                    make_argb(0xff, 200, 100, 50)
                } else {
                    make_argb(0xff, 10, 20, 30)
                }
            })
            .collect();
        round_trip(&img, w, h);
    }

    #[test]
    fn round_trips_two_color_palette() {
        // A 2-color image bundles 8 pixels per byte (width_bits = 3).
        let (a, b) = (make_argb(0xff, 0, 0, 0), make_argb(0xff, 255, 255, 255));
        let img: Vec<u32> = (0..30).map(|i| if i % 3 == 0 { a } else { b }).collect();
        round_trip(&img, 6, 5);
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
