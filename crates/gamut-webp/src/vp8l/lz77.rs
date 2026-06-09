//! VP8L LZ77 backward references (RFC 9649 §3.6.2).
//!
//! Beyond literal ARGB pixels, VP8L codes `(length, distance)` backward references into the pixels
//! already produced. Both length and distance are stored with **prefix coding**: a Huffman-coded
//! prefix code selects a value range, and the low bits within that range are stored raw (see
//! [`read_lz77_value`]). Distances are additionally remapped — the smallest codes (1..=120) name a
//! pixel in a fixed 2-D neighborhood ([`DISTANCE_MAP`]); larger codes are a plain scan-line distance
//! offset by 120 (see [`distance_code_to_pixel_distance`]).

use gamut_core::{Error, Result};

use crate::vp8l::bit_io::BitReader;

/// Maximum LZ77 copy length (RFC 9649 §5.2.2): only the first 24 length prefix codes are meaningful.
pub const MAX_COPY_LENGTH: u32 = 4096;

/// The neighborhood for the 120 smallest distance codes: `(xi, yi)` offsets in scan order
/// (RFC 9649 §3.6.2). Distance code `c` (1-based) maps to `DISTANCE_MAP[c - 1]`.
pub const DISTANCE_MAP: [(i8, i8); 120] = [
    (0, 1),
    (1, 0),
    (1, 1),
    (-1, 1),
    (0, 2),
    (2, 0),
    (1, 2),
    (-1, 2),
    (2, 1),
    (-2, 1),
    (2, 2),
    (-2, 2),
    (0, 3),
    (3, 0),
    (1, 3),
    (-1, 3),
    (3, 1),
    (-3, 1),
    (2, 3),
    (-2, 3),
    (3, 2),
    (-3, 2),
    (0, 4),
    (4, 0),
    (1, 4),
    (-1, 4),
    (4, 1),
    (-4, 1),
    (3, 3),
    (-3, 3),
    (2, 4),
    (-2, 4),
    (4, 2),
    (-4, 2),
    (0, 5),
    (3, 4),
    (-3, 4),
    (4, 3),
    (-4, 3),
    (5, 0),
    (1, 5),
    (-1, 5),
    (5, 1),
    (-5, 1),
    (2, 5),
    (-2, 5),
    (5, 2),
    (-5, 2),
    (4, 4),
    (-4, 4),
    (3, 5),
    (-3, 5),
    (5, 3),
    (-5, 3),
    (0, 6),
    (6, 0),
    (1, 6),
    (-1, 6),
    (6, 1),
    (-6, 1),
    (2, 6),
    (-2, 6),
    (6, 2),
    (-6, 2),
    (4, 5),
    (-4, 5),
    (5, 4),
    (-5, 4),
    (3, 6),
    (-3, 6),
    (6, 3),
    (-6, 3),
    (0, 7),
    (7, 0),
    (1, 7),
    (-1, 7),
    (5, 5),
    (-5, 5),
    (7, 1),
    (-7, 1),
    (4, 6),
    (-4, 6),
    (6, 4),
    (-6, 4),
    (2, 7),
    (-2, 7),
    (7, 2),
    (-7, 2),
    (3, 7),
    (-3, 7),
    (7, 3),
    (-7, 3),
    (5, 6),
    (-5, 6),
    (6, 5),
    (-6, 5),
    (8, 0),
    (4, 7),
    (-4, 7),
    (7, 4),
    (-7, 4),
    (8, 1),
    (8, 2),
    (6, 6),
    (-6, 6),
    (8, 3),
    (5, 7),
    (-5, 7),
    (7, 5),
    (-7, 5),
    (8, 4),
    (6, 7),
    (-6, 7),
    (7, 6),
    (-7, 6),
    (8, 5),
    (7, 7),
    (-7, 7),
    (8, 6),
    (8, 7),
];

/// The threshold below which a distance code names a [`DISTANCE_MAP`] neighbor rather than a plain
/// scan-line distance.
pub const DISTANCE_MAP_LEN: u32 = 120;

/// Decodes a length-or-distance value from its `prefix_code` plus any raw extra bits (RFC 9649
/// §5.2.2).
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if the prefix code implies an absurd number of extra bits or the
/// stream is truncated.
pub fn read_lz77_value(r: &mut BitReader<'_>, prefix_code: u32) -> Result<u32> {
    if prefix_code < 4 {
        return Ok(prefix_code + 1);
    }
    let extra_bits = (prefix_code - 2) >> 1;
    if extra_bits > 24 {
        return Err(Error::InvalidInput("VP8L: LZ77 prefix code out of range"));
    }
    let offset = (2 + (prefix_code & 1)) << extra_bits;
    Ok(offset + r.read_bits(extra_bits)? + 1)
}

/// Converts a 1-based `distance_code` to a backward pixel distance for an image of `width` pixels
/// (RFC 9649 §3.6.2). Codes `> 120` are `distance_code - 120`; smaller codes index [`DISTANCE_MAP`].
/// The result is clamped to at least 1.
#[must_use]
pub fn distance_code_to_pixel_distance(distance_code: u32, width: u32) -> u32 {
    if distance_code > DISTANCE_MAP_LEN {
        return distance_code - DISTANCE_MAP_LEN;
    }
    if distance_code == 0 {
        return 1; // defensive: the value formula never yields 0
    }
    let (xi, yi) = DISTANCE_MAP
        .get((distance_code - 1) as usize)
        .copied()
        .unwrap_or((0, 1));
    let dist = i32::from(xi) + i32::from(yi) * width as i32;
    if dist < 1 { 1 } else { dist as u32 }
}

// --- Encoder side ---------------------------------------------------------------------------------

/// Minimum backward-reference length the matcher will emit; shorter runs are cheaper as literals or
/// cache codes.
pub const MIN_MATCH: usize = 3;
/// Hash-table width (number of chain heads is `1 << HASH_BITS`).
const HASH_BITS: u32 = 14;
/// Maximum chain length walked per position — the search-depth knob, deferred for tuning to #31.
const MAX_CHAIN: usize = 32;

/// Splits a length-or-distance `value` into `(prefix_code, num_extra_bits, extra_value)` — the exact
/// inverse of [`read_lz77_value`] (RFC 9649 §5.2.2).
#[must_use]
pub fn value_to_prefix(value: u32) -> (u16, u8, u32) {
    if value <= 4 {
        return ((value - 1) as u16, 0, 0);
    }
    let v = value - 1;
    let log = 31 - v.leading_zeros(); // floor(log2(v))
    let extra_bits = log - 1;
    let (prefix_code, offset) = if v < (3u32 << extra_bits) {
        (2 * extra_bits + 2, 1u32 << (extra_bits + 1))
    } else {
        (2 * extra_bits + 3, 3u32 << extra_bits)
    };
    (prefix_code as u16, extra_bits as u8, v - offset)
}

/// Maps a backward pixel `distance` to the smallest distance *code* for an image of `width` pixels:
/// a [`DISTANCE_MAP`] neighbor code when one matches exactly (post-clamp), else `distance + 120`
/// (RFC 9649 §3.6.2). It is the inverse of [`distance_code_to_pixel_distance`].
#[must_use]
pub fn pixel_distance_to_code(distance: u32, width: u32) -> u32 {
    for (i, &(xi, yi)) in DISTANCE_MAP.iter().enumerate() {
        let mapped = i32::from(xi) + i32::from(yi) * width as i32;
        let mapped = if mapped < 1 { 1 } else { mapped as u32 };
        if mapped == distance {
            return (i + 1) as u32;
        }
    }
    distance + DISTANCE_MAP_LEN
}

/// A hash-chain index over already-seen pixels for finding LZ77 backward references. Correctness
/// does not depend on the hash quality — matches are verified by comparison — only compression does.
pub struct BackwardRefs {
    /// Most recent position for each hash bucket (`-1` = empty).
    head: Vec<i32>,
    /// Previous position sharing a position's hash bucket (the chain).
    prev: Vec<i32>,
}

impl BackwardRefs {
    /// Creates an index for an image of `num_pixels` pixels.
    #[must_use]
    pub fn new(num_pixels: usize) -> Self {
        Self {
            head: vec![-1; 1usize << HASH_BITS],
            prev: vec![-1; num_pixels],
        }
    }

    /// Hashes the `MIN_MATCH`-pixel window at `pos` (caller guarantees the window is in bounds).
    fn hash(pixels: &[u32], pos: usize) -> usize {
        let a = u64::from(pixels[pos]);
        let b = u64::from(pixels[pos + 1]);
        let c = u64::from(pixels[pos + 2]);
        let mix =
            a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA77) ^ c.wrapping_mul(0xC2B2_AE3D);
        ((mix >> (64 - HASH_BITS)) as usize) & ((1usize << HASH_BITS) - 1)
    }

    /// Records `pos` as a match candidate for future positions.
    pub fn insert(&mut self, pixels: &[u32], pos: usize) {
        if pos + MIN_MATCH > pixels.len() {
            return;
        }
        let h = Self::hash(pixels, pos);
        self.prev[pos] = self.head[h];
        self.head[h] = pos as i32;
    }

    /// Finds the longest backward reference for the pixels starting at `pos`, or `None` if none
    /// reaches [`MIN_MATCH`]. Returns `(length, distance)` with `1 <= distance <= pos`.
    #[must_use]
    pub fn find(&self, pixels: &[u32], pos: usize) -> Option<(u32, u32)> {
        let n = pixels.len();
        if pos + MIN_MATCH > n {
            return None;
        }
        let max_len = (n - pos).min(MAX_COPY_LENGTH as usize);
        let mut candidate = self.head[Self::hash(pixels, pos)];
        let mut best_len = 0usize;
        let mut best_dist = 0u32;
        let mut chain = 0usize;
        while candidate >= 0 && chain < MAX_CHAIN {
            let c = candidate as usize;
            let mut len = 0usize;
            // Overlapping copies (source running into the destination) are valid for run-length.
            while len < max_len && pixels[c + len] == pixels[pos + len] {
                len += 1;
            }
            if len > best_len {
                best_len = len;
                best_dist = (pos - c) as u32;
            }
            candidate = self.prev[c];
            chain += 1;
        }
        if best_len >= MIN_MATCH {
            Some((best_len as u32, best_dist))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vp8l::bit_io::BitWriter;

    #[test]
    fn distance_map_has_120_entries() {
        assert_eq!(DISTANCE_MAP.len(), 120);
    }

    #[test]
    fn distance_code_examples_from_spec() {
        // Code 1 -> (0, 1) -> the pixel directly above (distance = width).
        assert_eq!(distance_code_to_pixel_distance(1, 16), 16);
        // Code 3 -> (1, 1) -> top-left pixel (distance = 1 + width).
        assert_eq!(distance_code_to_pixel_distance(3, 16), 17);
        // Code 2 -> (1, 0) -> the pixel to the left (distance 1).
        assert_eq!(distance_code_to_pixel_distance(2, 16), 1);
        // Codes above 120 are plain scan-line distances offset by 120.
        assert_eq!(distance_code_to_pixel_distance(121, 16), 1);
        assert_eq!(distance_code_to_pixel_distance(500, 16), 380);
    }

    #[test]
    fn distance_clamps_to_one() {
        // Code 4 -> (-1, 1): at width 1 the raw distance is 0, clamped up to 1.
        assert_eq!(distance_code_to_pixel_distance(4, 1), 1);
    }

    #[test]
    fn lz77_value_small_codes_have_no_extra_bits() {
        let mut r = BitReader::new(&[]);
        assert_eq!(read_lz77_value(&mut r, 0).unwrap(), 1);
        assert_eq!(read_lz77_value(&mut r, 1).unwrap(), 2);
        assert_eq!(read_lz77_value(&mut r, 2).unwrap(), 3);
        assert_eq!(read_lz77_value(&mut r, 3).unwrap(), 4);
    }

    #[test]
    fn lz77_value_uses_extra_bits() {
        // prefix_code 4: extra_bits = 1, offset = 2<<1 = 4, value = 4 + extra + 1.
        for (extra, expected) in [(0u32, 5u32), (1, 6)] {
            let mut w = BitWriter::new();
            w.write_bits(extra, 1);
            let bytes = w.finish();
            let mut r = BitReader::new(&bytes);
            assert_eq!(read_lz77_value(&mut r, 4).unwrap(), expected);
        }
        // prefix_code 7: extra_bits = 2, offset = (2 + 1) << 2 = 12, value = 12 + extra + 1.
        let mut w = BitWriter::new();
        w.write_bits(3, 2); // extra = 3
        let bytes = w.finish();
        let mut r = BitReader::new(&bytes);
        assert_eq!(read_lz77_value(&mut r, 7).unwrap(), 12 + 3 + 1);
    }

    #[test]
    fn value_to_prefix_inverts_read_lz77_value() {
        let check = |v: u32| {
            let (prefix_code, num_extra, extra) = value_to_prefix(v);
            let mut w = BitWriter::new();
            w.write_bits(extra, u32::from(num_extra));
            let bytes = w.finish();
            let mut r = BitReader::new(&bytes);
            assert_eq!(
                read_lz77_value(&mut r, u32::from(prefix_code)).unwrap(),
                v,
                "value {v} did not round-trip"
            );
        };
        for v in 1..=MAX_COPY_LENGTH {
            check(v);
        }
        for v in [4097, 10_000, 65_536, 100_000, 524_288, 786_432, 1_048_576] {
            check(v);
        }
    }

    #[test]
    fn pixel_distance_to_code_inverts_mapping() {
        for width in [1u32, 16, 100] {
            for dist in 1..=300u32 {
                let code = pixel_distance_to_code(dist, width);
                assert_eq!(
                    distance_code_to_pixel_distance(code, width),
                    dist,
                    "dist {dist}"
                );
            }
        }
    }

    #[test]
    fn matcher_finds_repeated_run() {
        // A repeated 8-pixel block: the matcher should find a backward reference at distance 8.
        let block = [1u32, 2, 3, 4, 5, 6, 7, 8];
        let mut pixels = block.to_vec();
        pixels.extend_from_slice(&block);
        let mut refs = BackwardRefs::new(pixels.len());
        for p in 0..8 {
            refs.insert(&pixels, p);
        }
        let (len, dist) = refs.find(&pixels, 8).expect("match");
        assert_eq!(dist, 8);
        assert_eq!(len, 8);
    }
}
