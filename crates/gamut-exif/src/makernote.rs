//! Vendor-specific MakerNote blocks.
//!
//! The `MakerNote` tag (`0x927C`) carries an opaque, vendor-defined block — usually an IFD with
//! per-vendor quirks (its own byte order, offset base, or header). MakerNotes are the largest source
//! of EXIF tag breadth and the main lever for exiftool parity; per-vendor decoding is added
//! incrementally during implementation, dispatched on the detected [`MakerNoteVendor`]. Kept in this
//! crate (rather than a sub-crate) for now; extraction is an option if the vendor set grows large.

use gamut_ifd::Ifd;

/// A decoded MakerNote block.
pub struct MakerNote {
    /// The vendor whose dialect the block follows.
    pub vendor: MakerNoteVendor,
    /// The decoded vendor IFD (most MakerNotes are IFD-structured, though offset conventions vary).
    pub ifd: Ifd,
}

/// The MakerNote vendor dialect, detected from the `Make` tag and the block's signature.
/// Representative set; more vendors are added during implementation.
pub enum MakerNoteVendor {
    /// Canon.
    Canon,
    /// Nikon (types 1–3).
    Nikon,
    /// Sony.
    Sony,
    /// Fujifilm.
    Fujifilm,
    /// Olympus / OM Digital.
    Olympus,
    /// Panasonic / Lumix.
    Panasonic,
    /// Apple.
    Apple,
    /// An unrecognised or undocumented dialect — preserved as raw bytes.
    Unknown,
}
