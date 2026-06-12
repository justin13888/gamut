//! The byte order of a TIFF/IFD stream.

/// Byte order of a TIFF stream, named by the two-byte order mark at the start of the header.
///
/// Every scalar in the stream — IFD offsets, entry counts, and multi-byte values — is read and
/// written in this order. Unlike most binary image formats (which fix one endianness), TIFF
/// records its own, so a reader must thread the [`ByteOrder`] through every access.
pub enum ByteOrder {
    /// Little-endian (`II`, `0x4949`): least-significant byte first.
    LittleEndian,
    /// Big-endian (`MM`, `0x4D4D`): most-significant byte first.
    BigEndian,
}
