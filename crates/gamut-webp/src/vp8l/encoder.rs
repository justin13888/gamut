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
//! [`build_length_limited_lengths`] and written
//! with the normal code-length code. Single-symbol codes (e.g. a solid color, or the unused distance
//! code) consume no bits, exactly as the decoder expects.

use gamut_core::{Dimensions, Error, Result};

use crate::vp8l::bit_io::BitWriter;
use crate::vp8l::color_cache::ColorCache;
use crate::vp8l::div_round_up;
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

/// Block-size exponent for the meta-prefix (entropy) image (16×16 meta-blocks).
const PREFIX_BITS: u32 = 4;
/// Cap on prefix-code groups; beyond this the encoder falls back to a single group rather than pay
/// the per-group overhead (a naive bound — smarter clustering is deferred to issue #31).
const MAX_GROUPS: u32 = 256;

/// One coding decision for the entropy-coded pixel stream.
enum Token {
    /// A literal ARGB pixel (green, red, blue, alpha symbols).
    Literal(u32),
    /// An LZ77 backward reference: copy `len` pixels from `dist` pixels back.
    Copy { len: u32, dist: u32 },
    /// A color-cache hit at the given slot.
    CacheIndex(u16),
}

impl Token {
    /// Number of output pixels this token produces.
    fn pixel_count(&self) -> usize {
        match self {
            Token::Copy { len, .. } => *len as usize,
            Token::Literal(_) | Token::CacheIndex(_) => 1,
        }
    }

    /// The green-alphabet symbol this token codes (literal green, length code, or cache index) —
    /// used as a cheap per-block signature for grouping.
    fn green_symbol(&self) -> usize {
        match *self {
            Token::Literal(p) => green(p) as usize,
            Token::Copy { len, .. } => LENGTH_CODE_BASE + value_to_prefix(len).0 as usize,
            Token::CacheIndex(idx) => CACHE_CODE_BASE + idx as usize,
        }
    }
}

/// Encodes an ARGB image (scan order, `0xAARRGGBB`) to a VP8L bitstream body — the bytes that go
/// inside the `VP8L` chunk, signature byte included.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if `argb.len()` does not equal `width * height`, or the
/// dimensions are out of the VP8L range.
pub fn encode(argb: &[u32], dims: Dimensions) -> Result<Vec<u8>> {
    check_dimensions(argb, dims)?;
    let alpha_is_used = argb.iter().any(|&p| alpha(p) != 0xff);
    let header = Vp8lHeader::from_dimensions(dims, alpha_is_used)?;
    let mut w = BitWriter::new();
    header.write(&mut w);
    write_image_body(&mut w, argb, dims);
    Ok(w.finish())
}

/// Encodes an ARGB image to a **headerless** VP8L image-stream — the transform chain plus the
/// spatially-coded image, without the dimension-carrying header. This is the form a
/// lossless-compressed `ALPH` chunk takes (RFC 9649 §2.7.1, implicit dimensions); the header in a
/// full [`encode`] is exactly five (byte-aligned) bytes, so this is that output minus those bytes.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if `argb.len()` does not equal `width * height`.
pub fn encode_image(argb: &[u32], dims: Dimensions) -> Result<Vec<u8>> {
    check_dimensions(argb, dims)?;
    let mut w = BitWriter::new();
    write_image_body(&mut w, argb, dims);
    Ok(w.finish())
}

/// Validates that `argb` holds exactly `width * height` pixels.
fn check_dimensions(argb: &[u32], dims: Dimensions) -> Result<()> {
    if (dims.width as usize).checked_mul(dims.height as usize) == Some(argb.len()) {
        Ok(())
    } else {
        Err(Error::InvalidInput(
            "VP8L: pixel buffer does not match dimensions",
        ))
    }
}

/// Writes the image body (transforms + spatially-coded image). Few-color images take the palette
/// path; everything else takes the spatial-transform path. Choosing the densest path for a given
/// image is a tuning concern deferred to issue #31.
fn write_image_body(w: &mut BitWriter, argb: &[u32], dims: Dimensions) {
    match build_palette(argb) {
        Some(palette) => encode_palette(w, argb, dims, &palette),
        None => encode_spatial(w, argb, dims),
    }
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

/// Per-channel histograms used to build one prefix-code group.
struct Histograms {
    green: Vec<u32>,
    red: Vec<u32>,
    blue: Vec<u32>,
    alpha: Vec<u32>,
    distance: Vec<u32>,
}

impl Histograms {
    fn new(cache_size: usize) -> Self {
        Self {
            green: vec![0; green_alphabet_size(cache_size)],
            red: vec![0; NUM_LITERAL_CODES],
            blue: vec![0; NUM_LITERAL_CODES],
            alpha: vec![0; NUM_LITERAL_CODES],
            distance: vec![0; NUM_DISTANCE_CODES],
        }
    }

    /// Accumulates a token's symbols.
    fn add(&mut self, token: &Token, width: u32) {
        match *token {
            Token::Literal(p) => {
                self.green[green(p) as usize] += 1;
                self.red[red(p) as usize] += 1;
                self.blue[blue(p) as usize] += 1;
                self.alpha[alpha(p) as usize] += 1;
            }
            Token::Copy { len, dist } => {
                self.green[LENGTH_CODE_BASE + value_to_prefix(len).0 as usize] += 1;
                let dist_code = pixel_distance_to_code(dist, width);
                self.distance[value_to_prefix(dist_code).0 as usize] += 1;
            }
            Token::CacheIndex(idx) => self.green[CACHE_CODE_BASE + idx as usize] += 1,
        }
    }

    /// Builds the five prefix codes.
    fn build(&self) -> CodeGroup {
        CodeGroup {
            green: build_code(&self.green),
            red: build_code(&self.red),
            blue: build_code(&self.blue),
            alpha: build_code(&self.alpha),
            distance: build_code(&self.distance),
        }
    }
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

    /// Emits one token's symbols (and any LZ77 extra bits).
    fn write_token(&self, w: &mut BitWriter, token: &Token, width: u32) {
        match *token {
            Token::Literal(p) => {
                self.green.write_symbol(w, green(p) as usize);
                self.red.write_symbol(w, red(p) as usize);
                self.blue.write_symbol(w, blue(p) as usize);
                self.alpha.write_symbol(w, alpha(p) as usize);
            }
            Token::Copy { len, dist } => {
                let (len_code, len_bits, len_extra) = value_to_prefix(len);
                self.green
                    .write_symbol(w, LENGTH_CODE_BASE + len_code as usize);
                w.write_bits(len_extra, u32::from(len_bits));
                let (dist_sym, dist_bits, dist_extra) =
                    value_to_prefix(pixel_distance_to_code(dist, width));
                self.distance.write_symbol(w, dist_sym as usize);
                w.write_bits(dist_extra, u32::from(dist_bits));
            }
            Token::CacheIndex(idx) => self.green.write_symbol(w, CACHE_CODE_BASE + idx as usize),
        }
    }
}

/// Writes a sub-resolution image (predictor/color sub-images, palette, entropy image) as literal
/// pixels: no color cache, no meta prefix codes (the decoder reads these with `allow_meta = false`).
fn write_sub_image(w: &mut BitWriter, pixels: &[u32]) {
    w.write_bits(0, 1); // No color cache.
    let mut hist = Histograms::new(0);
    for &p in pixels {
        hist.add(&Token::Literal(p), 0);
    }
    let codes = hist.build();
    codes.write_descriptions(w);
    for &p in pixels {
        codes.write_token(w, &Token::Literal(p), 0);
    }
}

/// Writes the top-level image with a color cache, LZ77 backward references, and (when the encoder
/// splits the image into multiple statistical regions) a meta-prefix entropy image.
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

    let tokens = tokenize(pixels, cache_bits);
    let height = (pixels.len() as u32).checked_div(width).unwrap_or(0);
    let groups = assign_groups(&tokens, width, height);

    if groups.num_groups > 1 {
        w.write_bits(1, 1); // Meta prefix codes present.
        w.write_bits(groups.prefix_bits - 2, 3);
        write_sub_image(w, &groups.entropy_image());
    } else {
        w.write_bits(0, 1); // Single meta prefix code.
    }

    // Histogram tokens into their groups (a copy's symbols use its start-position group), build a
    // code group each, emit the descriptions, then replay the tokens.
    let mut histograms: Vec<Histograms> = (0..groups.num_groups)
        .map(|_| Histograms::new(cache_size))
        .collect();
    let mut pos = 0usize;
    for token in &tokens {
        histograms[groups.group_at(pos, width)].add(token, width);
        pos += token.pixel_count();
    }
    let code_groups: Vec<CodeGroup> = histograms.iter().map(Histograms::build).collect();
    for group in &code_groups {
        group.write_descriptions(w);
    }
    let mut pos = 0usize;
    for token in &tokens {
        code_groups[groups.group_at(pos, width)].write_token(w, token, width);
        pos += token.pixel_count();
    }
}

/// The assignment of image meta-blocks to prefix-code groups.
struct GroupAssignment {
    prefix_bits: u32,
    grid_width: u32,
    block_group: Vec<u32>,
    num_groups: u32,
}

impl GroupAssignment {
    /// The group for the meta-block containing pixel index `pos`.
    fn group_at(&self, pos: usize, width: u32) -> usize {
        if self.num_groups <= 1 || width == 0 {
            return 0;
        }
        let x = pos as u32 % width;
        let y = pos as u32 / width;
        let block = (y >> self.prefix_bits) * self.grid_width + (x >> self.prefix_bits);
        self.block_group.get(block as usize).copied().unwrap_or(0) as usize
    }

    /// Builds the entropy image: one pixel per meta-block with the group id in the green channel.
    fn entropy_image(&self) -> Vec<u32> {
        self.block_group
            .iter()
            .map(|&g| crate::vp8l::transform::make_argb(0xff, 0, g as u8, 0))
            .collect()
    }
}

/// Assigns meta-blocks to groups by a cheap signature (each block's most frequent green symbol).
/// Distinct signatures become distinct groups; if every block matches, a single group is used (no
/// entropy-image overhead). Smarter, cost-aware clustering is deferred to issue #31.
fn assign_groups(tokens: &[Token], width: u32, height: u32) -> GroupAssignment {
    use std::collections::HashMap;
    let grid_width = div_round_up(width, 1 << PREFIX_BITS);
    let grid_height = div_round_up(height, 1 << PREFIX_BITS);
    let num_blocks = (grid_width as usize) * (grid_height as usize);
    let single = GroupAssignment {
        prefix_bits: PREFIX_BITS,
        grid_width,
        block_group: vec![0; num_blocks.max(1)],
        num_groups: 1,
    };
    if num_blocks <= 1 || width == 0 {
        return single;
    }

    // Per-block green-symbol counts → most frequent symbol as the block signature.
    let mut counts: Vec<HashMap<usize, u32>> = (0..num_blocks).map(|_| HashMap::new()).collect();
    let mut pos = 0usize;
    for token in tokens {
        let x = pos as u32 % width;
        let y = pos as u32 / width;
        let block = ((y >> PREFIX_BITS) * grid_width + (x >> PREFIX_BITS)) as usize;
        if let Some(counter) = counts.get_mut(block) {
            *counter.entry(token.green_symbol()).or_insert(0) += 1;
        }
        pos += token.pixel_count();
    }

    let mut signature_group: HashMap<usize, u32> = HashMap::new();
    let mut block_group = vec![0u32; num_blocks];
    let mut num_groups = 0u32;
    for (block, counter) in counts.iter().enumerate() {
        let signature = counter
            .iter()
            .max_by_key(|&(_, &count)| count)
            .map_or(0, |(&sym, _)| sym);
        let group = match signature_group.get(&signature) {
            Some(&g) => g,
            None => {
                let id = num_groups;
                signature_group.insert(signature, id);
                num_groups += 1;
                id
            }
        };
        block_group[block] = group;
    }

    if num_groups <= 1 || num_groups > MAX_GROUPS {
        return single;
    }
    GroupAssignment {
        prefix_bits: PREFIX_BITS,
        grid_width,
        block_group,
        num_groups,
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
    fn distinct_regions_use_multiple_groups() {
        // A 32-color image (palette path, no bundling) whose top and bottom halves draw their
        // indices from disjoint ranges, scattered so they stay literals rather than LZ77 runs. The
        // first-seen palette order maps the top colors to low indices and the bottom colors to high
        // ones, so the meta-blocks have clearly different green statistics → multiple groups.
        let palette: Vec<u32> = (0..32)
            .map(|i| make_argb(0xff, i as u8, (i * 7) as u8, (i * 13) as u8))
            .collect();
        let (w, h) = (32u32, 32u32);
        let img: Vec<u32> = (0..(w * h))
            .map(|i| {
                let (x, y) = ((i % w) as usize, (i / w) as usize);
                let scatter = (x * 7 + y * 11) % 16;
                let idx = if y < 16 { scatter } else { 16 + scatter };
                palette[idx]
            })
            .collect();

        // Replicate the encoder's internal grouping to assert it splits into multiple groups.
        let detected = build_palette(&img).expect("few-color image has a palette");
        let (bundled, bundled_width) = forward_color_indexing(&img, w, h, &detected);
        let tokens = tokenize(&bundled, pick_cache_bits(bundled.len()));
        let assignment =
            assign_groups(&tokens, bundled_width, bundled.len() as u32 / bundled_width);
        assert!(
            assignment.num_groups >= 2,
            "expected multiple groups, got {}",
            assignment.num_groups
        );

        round_trip(&img, w, h);
    }

    #[test]
    fn assign_groups_merges_uniform_blocks() {
        // All-literal tokens with the same green symbol must collapse to a single group.
        let tokens: Vec<Token> = (0..1024)
            .map(|_| Token::Literal(make_argb(0xff, 0, 7, 0)))
            .collect();
        let assignment = assign_groups(&tokens, 32, 32);
        assert_eq!(assignment.num_groups, 1);
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
