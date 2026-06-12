//! The IFD itself: the header, its tag entries, and the directory that holds them.

use crate::byte_order::ByteOrder;
use crate::value::Value;

/// The 8-byte TIFF header that opens the stream.
///
/// Layout: the 2-byte byte-order mark, the 2-byte magic number `42`, then the 4-byte offset of the
/// first IFD (measured from the start of the stream). EXIF reuses this header verbatim after its
/// `Exif\0\0` marker.
pub struct TiffHeader {
    /// The byte order all scalars in the stream are encoded in.
    pub byte_order: ByteOrder,
    /// Offset of the first IFD from the start of the stream.
    pub first_ifd_offset: u64,
}

/// One entry (field) in an Image File Directory: a tag paired with its decoded value.
///
/// On disk this is 12 bytes — tag (2), field type (2), value count (4), and a value-or-offset word
/// (4) — but once decoded only the tag and the resolved [`Value`] matter; the field type and count
/// are recoverable from the value.
pub struct Field {
    /// The 16-bit tag identifying the field (e.g. `256` for `ImageWidth`).
    pub tag: u16,
    /// The decoded value(s) of the field.
    pub value: Value,
}

/// A parsed Image File Directory — one node in the IFD chain (a TIFF page, or an EXIF/GPS/Interop
/// sub-directory).
///
/// On disk an IFD is a 2-byte entry count, its entries sorted in ascending tag order, then a 4-byte
/// offset to the next IFD (`0` if last). Sub-IFDs (the Exif/GPS/Interoperability directories) are
/// reached through a tag whose value is an offset into another IFD.
pub struct Ifd {
    /// The directory's fields, in ascending tag order.
    pub fields: Vec<Field>,
}
