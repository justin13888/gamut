//! RIFF chunk layout: an 8-byte header (FourCC + `uint32` little-endian size) followed by a payload
//! that is padded to an even length with a single zero byte (RFC 9649 §2.3; Google *WebP
//! Container*, "RIFF File Format").

use crate::fourcc::FourCc;

/// Size of a chunk header (FourCC + size field), in bytes.
pub const CHUNK_HEADER_LEN: usize = 8;

/// A parsed chunk header: its FourCC and the declared payload size (excluding header and padding).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkHeader {
    /// The chunk's four-character code.
    pub fourcc: FourCc,
    /// Declared payload size in bytes, not counting the header or any pad byte.
    pub size: u32,
}

impl ChunkHeader {
    /// Number of pad bytes that follow a payload of `size` bytes: RIFF pads an odd-sized payload to
    /// an even boundary with a single zero byte, so this is `1` for odd `size` and `0` otherwise.
    #[must_use]
    pub const fn padding(size: u32) -> usize {
        (size & 1) as usize
    }

    /// Total on-disk span of a chunk with this header: header + payload + padding, in bytes.
    #[must_use]
    pub const fn total_len(&self) -> usize {
        CHUNK_HEADER_LEN + self.size as usize + Self::padding(self.size)
    }
}

/// A borrowed RIFF chunk: its FourCC and a slice over its payload (excluding header and padding).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk<'a> {
    /// The chunk's four-character code.
    pub fourcc: FourCc,
    /// The chunk payload, excluding the header and any pad byte.
    pub payload: &'a [u8],
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padding_is_one_for_odd_sizes() {
        assert_eq!(ChunkHeader::padding(0), 0);
        assert_eq!(ChunkHeader::padding(4), 0);
        assert_eq!(ChunkHeader::padding(1), 1);
        assert_eq!(ChunkHeader::padding(7), 1);
    }

    #[test]
    fn total_len_includes_header_and_pad() {
        let even = ChunkHeader {
            fourcc: FourCc::VP8L,
            size: 10,
        };
        assert_eq!(even.total_len(), CHUNK_HEADER_LEN + 10);
        let odd = ChunkHeader {
            fourcc: FourCc::VP8,
            size: 11,
        };
        assert_eq!(odd.total_len(), CHUNK_HEADER_LEN + 11 + 1);
    }
}
