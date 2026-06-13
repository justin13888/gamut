//! Huffman coding for DEFLATE: bit-reversal and the fixed code table (RFC 1951 §3.2.6).
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

/// The fixed Huffman code for a literal `byte`, as `(code, bit_length)` (RFC 1951 §3.2.6).
///
/// `code` is the canonical value read MSB-first; pass it through [`reverse_bits`] before emitting.
/// Literals 0–143 are 8 bits, 144–255 are 9 bits.
pub(crate) fn fixed_literal(byte: u8) -> (u32, u32) {
    match byte {
        0..=143 => (0x30 + u32::from(byte), 8),
        144..=255 => (0x190 + (u32::from(byte) - 144), 9),
    }
}

/// Fixed Huffman end-of-block symbol (256): the 7-bit canonical code `0`.
pub(crate) const FIXED_EOB_CODE: u32 = 0;
/// Bit length of the fixed end-of-block code.
pub(crate) const FIXED_EOB_LEN: u32 = 7;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_bits_examples() {
        assert_eq!(reverse_bits(0b001, 3), 0b100);
        assert_eq!(reverse_bits(0b1011, 4), 0b1101);
        assert_eq!(reverse_bits(0, 7), 0);
        assert_eq!(reverse_bits(0b1, 1), 0b1);
        // Higher bits beyond `len` are ignored.
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
    fn fixed_literal_boundaries() {
        assert_eq!(fixed_literal(0), (0x30, 8));
        assert_eq!(fixed_literal(143), (0xBF, 8));
        assert_eq!(fixed_literal(144), (0x190, 9));
        assert_eq!(fixed_literal(255), (0x1FF, 9));
    }
}
