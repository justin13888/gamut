//! Huffman coding for DEFLATE: bit-reversal and the fixed code tables (RFC 1951 §3.2.6).
//!
//! DEFLATE packs Huffman codes most-significant-bit-of-the-code first, but [`BitWriter`] is
//! LSB-first, so every code is bit-reversed via [`reverse_bits`] before emission. The canonical
//! dynamic-code builder (length-limited, package-merge) lands in a later phase.
//!
//! [`BitWriter`]: crate::bitwriter::BitWriter

/// Reverses the low `len` bits of `value`, discarding higher bits.
///
/// A Huffman code's canonical value is defined MSB-first; reversing its low `len` bits lets the
/// LSB-first [`BitWriter`](crate::bitwriter::BitWriter) emit it in the correct on-wire order.
pub(crate) fn reverse_bits(value: u32, len: u32) -> u32 {
    let mut v = value;
    let mut r = 0u32;
    for _ in 0..len {
        r = (r << 1) | (v & 1);
        v >>= 1;
    }
    r
}

/// The fixed Huffman code `(code, bit_length)` for a literal/length symbol (RFC 1951 §3.2.6).
///
/// Covers the whole literal/length alphabet 0..=287; callers pass only symbols that occur
/// (literals 0..=255, end-of-block 256, length symbols 257..=285). `code` is the canonical value
/// read MSB-first; pass it through [`reverse_bits`] before emitting.
pub(crate) fn fixed_litlen(sym: u16) -> (u32, u32) {
    match sym {
        0..=143 => (0x30 + u32::from(sym), 8),
        144..=255 => (0x190 + (u32::from(sym) - 144), 9),
        256..=279 => (u32::from(sym) - 256, 7),
        _ => (0xC0 + (u32::from(sym) - 280), 8), // 280..=287
    }
}

/// The fixed Huffman code `(code, bit_length)` for a distance symbol 0..=29: a 5-bit code equal to
/// the symbol number (RFC 1951 §3.2.6). MSB-first; reverse before emitting.
pub(crate) fn fixed_distance(sym: u16) -> (u32, u32) {
    (u32::from(sym), 5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_bits_examples() {
        assert_eq!(reverse_bits(0b001, 3), 0b100);
        assert_eq!(reverse_bits(0b1011, 4), 0b1101);
        assert_eq!(reverse_bits(0, 7), 0);
        assert_eq!(reverse_bits(0b1, 1), 0b1);
        // Bits beyond `len` are ignored.
        assert_eq!(reverse_bits(0b1_0000_0001, 1), 0b1);
    }

    #[test]
    fn reverse_is_an_involution() {
        for len in 1..=15u32 {
            for v in [0u32, 1, 5, 0x2A, (1 << len) - 1] {
                let masked = v & ((1 << len) - 1);
                assert_eq!(reverse_bits(reverse_bits(masked, len), len), masked);
            }
        }
    }

    #[test]
    fn fixed_litlen_boundaries() {
        assert_eq!(fixed_litlen(0), (0x30, 8));
        assert_eq!(fixed_litlen(143), (0xBF, 8));
        assert_eq!(fixed_litlen(144), (0x190, 9));
        assert_eq!(fixed_litlen(255), (0x1FF, 9));
        assert_eq!(fixed_litlen(256), (0, 7)); // end of block
        assert_eq!(fixed_litlen(279), (23, 7));
        assert_eq!(fixed_litlen(280), (0xC0, 8));
        assert_eq!(fixed_litlen(285), (0xC5, 8));
    }

    #[test]
    fn fixed_distance_is_five_bit_identity() {
        assert_eq!(fixed_distance(0), (0, 5));
        assert_eq!(fixed_distance(29), (29, 5));
    }
}
