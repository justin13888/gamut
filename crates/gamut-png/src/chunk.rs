//! PNG chunk framing and the file signature (PNG spec §5).
//!
//! Every chunk is `length (u32 BE) || type (4 bytes) || data || CRC-32 (u32 BE)`, where the CRC
//! covers the type and data. All multi-byte integers in PNG are big-endian — the opposite of the
//! DEFLATE/zlib payload the IDAT chunks carry.

use crate::crc32::Crc32;

/// The 8-byte PNG file signature (`\x89PNG\r\n\x1a\n`).
pub(crate) const SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// Appends a complete chunk (`length`, `type`, `data`, `CRC`) to `out`.
pub(crate) fn write_chunk(out: &mut Vec<u8>, chunk_type: [u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(&chunk_type);
    out.extend_from_slice(data);
    let mut crc = Crc32::new();
    crc.update(&chunk_type);
    crc.update(data);
    out.extend_from_slice(&crc.finish().to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iend_chunk_layout() {
        let mut out = Vec::new();
        write_chunk(&mut out, *b"IEND", &[]);
        // length 0, type "IEND", no data, fixed CRC 0xAE426082.
        assert_eq!(
            out,
            vec![0, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82]
        );
    }

    #[test]
    fn chunk_carries_length_and_data() {
        let mut out = Vec::new();
        write_chunk(&mut out, *b"tEXt", &[1, 2, 3]);
        assert_eq!(out[..4], 3u32.to_be_bytes()); // length field
        assert_eq!(&out[4..8], b"tEXt"); // type
        assert_eq!(&out[8..11], &[1, 2, 3]); // data
        assert_eq!(out.len(), 4 + 4 + 3 + 4); // + CRC
    }
}
