//! DEFLATE block writers. D1 implements stored (uncompressed) blocks; fixed- and dynamic-Huffman
//! blocks land in later phases.

use crate::bitwriter::BitWriter;

/// Maximum payload of a single stored block (`LEN` is a `u16`); larger inputs are split.
const MAX_STORED: usize = 0xFFFF;

/// Writes `data` as one or more stored (uncompressed) blocks (BTYPE = 00), splitting at 65535
/// bytes. `BFINAL` is set on the final block. Empty input emits one empty final block — the
/// shortest valid DEFLATE stream.
pub(crate) fn write_stored(w: &mut BitWriter, data: &[u8]) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_stored_block_layout() {
        let mut w = BitWriter::new();
        write_stored(&mut w, &[0x11, 0x22, 0x33]);
        // BFINAL=1, BTYPE=00 -> first byte 0x01; LEN=3 (LE), NLEN=!3 (LE), then the data.
        assert_eq!(
            w.finish(),
            vec![0x01, 0x03, 0x00, 0xFC, 0xFF, 0x11, 0x22, 0x33]
        );
    }

    #[test]
    fn empty_emits_one_final_block() {
        let mut w = BitWriter::new();
        write_stored(&mut w, &[]);
        assert_eq!(w.finish(), vec![0x01, 0x00, 0x00, 0xFF, 0xFF]);
    }

    #[test]
    fn splits_past_64k() {
        let data = vec![0u8; MAX_STORED + 10];
        let mut w = BitWriter::new();
        write_stored(&mut w, &data);
        let bytes = w.finish();
        // Two blocks: a 5-byte header each plus payloads (65535 then 10); only the second is final.
        assert_eq!(bytes.len(), 5 + MAX_STORED + 5 + 10);
        assert_eq!(bytes[0], 0x00, "first block must not be final");
        assert_eq!(bytes[5 + MAX_STORED], 0x01, "second block must be final");
    }
}
