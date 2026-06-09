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
}
