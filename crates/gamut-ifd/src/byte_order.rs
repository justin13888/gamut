//! The byte order of a TIFF/IFD stream.

/// Byte order of a TIFF stream, named by the two-byte order mark at the start of the header.
///
/// Every scalar in the stream — IFD offsets, entry counts, and multi-byte values — is read and
/// written in this order. Unlike most binary image formats (which fix one endianness), TIFF
/// records its own, so a reader must thread the [`ByteOrder`] through every access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ByteOrder {
    /// Little-endian (`II`, `0x4949`): least-significant byte first.
    LittleEndian,
    /// Big-endian (`MM`, `0x4D4D`): most-significant byte first.
    BigEndian,
}

impl ByteOrder {
    /// Decodes a 16-bit unsigned integer from two bytes in this order.
    #[must_use]
    pub fn u16(self, b: [u8; 2]) -> u16 {
        match self {
            ByteOrder::LittleEndian => u16::from_le_bytes(b),
            ByteOrder::BigEndian => u16::from_be_bytes(b),
        }
    }

    /// Decodes a 32-bit unsigned integer from four bytes in this order.
    #[must_use]
    pub fn u32(self, b: [u8; 4]) -> u32 {
        match self {
            ByteOrder::LittleEndian => u32::from_le_bytes(b),
            ByteOrder::BigEndian => u32::from_be_bytes(b),
        }
    }

    /// Decodes a 64-bit unsigned integer from eight bytes in this order (BigTIFF offsets/counts).
    #[cfg(feature = "bigtiff")]
    #[must_use]
    pub fn u64(self, b: [u8; 8]) -> u64 {
        match self {
            ByteOrder::LittleEndian => u64::from_le_bytes(b),
            ByteOrder::BigEndian => u64::from_be_bytes(b),
        }
    }

    /// Encodes a 16-bit unsigned integer to two bytes in this order.
    #[must_use]
    pub fn pack_u16(self, v: u16) -> [u8; 2] {
        match self {
            ByteOrder::LittleEndian => v.to_le_bytes(),
            ByteOrder::BigEndian => v.to_be_bytes(),
        }
    }

    /// Encodes a 32-bit unsigned integer to four bytes in this order.
    #[must_use]
    pub fn pack_u32(self, v: u32) -> [u8; 4] {
        match self {
            ByteOrder::LittleEndian => v.to_le_bytes(),
            ByteOrder::BigEndian => v.to_be_bytes(),
        }
    }

    /// Encodes a 64-bit unsigned integer to eight bytes in this order (BigTIFF offsets/counts).
    #[cfg(feature = "bigtiff")]
    #[must_use]
    pub fn pack_u64(self, v: u64) -> [u8; 8] {
        match self {
            ByteOrder::LittleEndian => v.to_le_bytes(),
            ByteOrder::BigEndian => v.to_be_bytes(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_order_roundtrips_integers() {
        for order in [ByteOrder::LittleEndian, ByteOrder::BigEndian] {
            assert_eq!(order.u16(order.pack_u16(0xABCD)), 0xABCD);
            assert_eq!(order.u32(order.pack_u32(0x0123_4567)), 0x0123_4567);
            #[cfg(feature = "bigtiff")]
            assert_eq!(
                order.u64(order.pack_u64(0x0123_4567_89AB_CDEF)),
                0x0123_4567_89AB_CDEF
            );
        }
        assert_eq!(ByteOrder::LittleEndian.pack_u16(0x00FF), [0xFF, 0x00]);
        assert_eq!(ByteOrder::BigEndian.pack_u16(0x00FF), [0x00, 0xFF]);
        #[cfg(feature = "bigtiff")]
        {
            assert_eq!(
                ByteOrder::LittleEndian.pack_u64(0xFF),
                [0xFF, 0, 0, 0, 0, 0, 0, 0]
            );
            assert_eq!(
                ByteOrder::BigEndian.pack_u64(0xFF),
                [0, 0, 0, 0, 0, 0, 0, 0xFF]
            );
        }
    }
}
