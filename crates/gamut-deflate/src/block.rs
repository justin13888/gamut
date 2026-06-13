//! DEFLATE block writers. D1–D2 implement stored (uncompressed) and fixed-Huffman blocks; LZ77
//! matching and dynamic Huffman land in later phases.
//!
//! Each public helper returns a complete, byte-aligned DEFLATE body (`BFINAL` set) so the encoder
//! can build a few candidates and keep the smallest.

use crate::bitwriter::BitWriter;
use crate::huffman;

/// Maximum payload of a single stored block (`LEN` is a `u16`); larger inputs are split.
const MAX_STORED: usize = 0xFFFF;

/// Encodes `data` as a complete stored (uncompressed) DEFLATE body.
pub(crate) fn stored(data: &[u8]) -> Vec<u8> {
    let mut w = BitWriter::new();
    write_stored(&mut w, data);
    w.finish()
}

/// Encodes `data` as a complete fixed-Huffman DEFLATE body (literals only — no back-references
/// until the LZ77 phase lands).
pub(crate) fn fixed(data: &[u8]) -> Vec<u8> {
    let mut w = BitWriter::new();
    write_fixed(&mut w, data);
    w.finish()
}

/// Writes `data` as one or more stored blocks (BTYPE = 00), splitting at 65535 bytes. `BFINAL` is
/// set on the final block. Empty input emits one empty final block — the shortest valid stream.
fn write_stored(w: &mut BitWriter, data: &[u8]) {
    if data.is_empty() {
        write_stored_block(w, &[], true);
        return;
    }
    let mut chunks = data.chunks(MAX_STORED).peekable();
    while let Some(chunk) = chunks.next() {
        write_stored_block(w, chunk, chunks.peek().is_none());
    }
}

fn write_stored_block(w: &mut BitWriter, chunk: &[u8], is_final: bool) {
    debug_assert!(chunk.len() <= MAX_STORED);
    w.write_bits(u32::from(is_final), 1); // BFINAL
    w.write_bits(0b00, 2); // BTYPE = 00 (stored)
    w.align_to_byte();
    let len = chunk.len() as u16;
    w.write_bytes(&len.to_le_bytes());
    w.write_bytes(&(!len).to_le_bytes()); // NLEN = one's complement of LEN
    w.write_bytes(chunk);
}

/// Writes `data` as a single fixed-Huffman block (BTYPE = 01): every byte as its fixed literal code
/// followed by the end-of-block symbol. `BFINAL` is set (one block covers the whole input).
fn write_fixed(w: &mut BitWriter, data: &[u8]) {
    w.write_bits(1, 1); // BFINAL = 1
    w.write_bits(0b01, 2); // BTYPE = 01 (fixed Huffman)
    for &b in data {
        let (code, len) = huffman::fixed_literal(b);
        w.write_bits(huffman::reverse_bits(code, len), len);
    }
    w.write_bits(
        huffman::reverse_bits(huffman::FIXED_EOB_CODE, huffman::FIXED_EOB_LEN),
        huffman::FIXED_EOB_LEN,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_stored_block_layout() {
        // BFINAL=1, BTYPE=00 -> first byte 0x01; LEN=3 (LE), NLEN=!3 (LE), then the data.
        assert_eq!(
            stored(&[0x11, 0x22, 0x33]),
            vec![0x01, 0x03, 0x00, 0xFC, 0xFF, 0x11, 0x22, 0x33]
        );
    }

    #[test]
    fn empty_emits_one_final_stored_block() {
        assert_eq!(stored(&[]), vec![0x01, 0x00, 0x00, 0xFF, 0xFF]);
    }

    #[test]
    fn stored_splits_past_64k() {
        let bytes = stored(&vec![0u8; MAX_STORED + 10]);
        // Two blocks: a 5-byte header each plus payloads (65535 then 10); only the second is final.
        assert_eq!(bytes.len(), 5 + MAX_STORED + 5 + 10);
        assert_eq!(bytes[0], 0x00, "first block must not be final");
        assert_eq!(bytes[5 + MAX_STORED], 0x01, "second block must be final");
    }

    #[test]
    fn empty_fixed_block_is_header_plus_eob() {
        // BFINAL=1 (bit 0), BTYPE=01 (bits 1-2) -> low 3 bits 0b011; then the 7-bit EOB code 0.
        // 3 + 7 = 10 bits -> 2 bytes. The first byte's low 3 bits are 011 = 0x03; rest zero.
        assert_eq!(fixed(&[]), vec![0x03, 0x00]);
    }
}
