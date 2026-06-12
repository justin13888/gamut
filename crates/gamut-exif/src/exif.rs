//! The parsed EXIF model: the IFD chain that makes up an EXIF blob.

use gamut_ifd::{ByteOrder, Ifd};

/// A parsed EXIF blob — the directories of the TIFF stream that follows the `Exif\0\0` marker.
///
/// The 0th IFD holds the primary-image tags and the pointer tags that reach the Exif/GPS/Interop
/// sub-IFDs; the 1st IFD holds the thumbnail. The byte order is preserved so a re-serialized blob
/// can match the source's endianness. This scaffold models the structural skeleton over
/// [`gamut_ifd::Ifd`]; typed per-tag accessors are added during implementation.
pub struct Exif {
    /// The byte order of the underlying TIFF stream (preserved for round-tripping).
    pub byte_order: ByteOrder,
    /// The 0th IFD — primary-image / TIFF tags.
    pub image: Ifd,
    /// The Exif sub-IFD (capture parameters), if present.
    pub exif: Option<Ifd>,
    /// The GPS sub-IFD, if present.
    pub gps: Option<Ifd>,
    /// The Interoperability sub-IFD, if present.
    pub interop: Option<Ifd>,
    /// The 1st IFD — the embedded thumbnail's directory, if present.
    pub thumbnail: Option<Ifd>,
}
