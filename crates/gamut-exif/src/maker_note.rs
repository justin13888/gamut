//! The vendor-specific `MakerNote` block (Exif tag `0x927C`).
//!
//! `MakerNote` is an `UNDEFINED` blob of vendor-proprietary data — usually a mini-IFD with
//! per-vendor quirks (its own byte order, header, and offset base). v1 treats it as **opaque**: the
//! bytes are preserved verbatim and the vendor is *detected* (from the `Make` tag), but the block is
//! **not decoded**. Per-vendor decoding is deferred; it can be added later without breaking this
//! API (hence [`MakerNoteVendor`] is `#[non_exhaustive]`).
//!
//! # Round-trip behaviour
//!
//! The *bytes* always round-trip exactly (value-level). Additionally, a model that came from
//! [`Exif::parse`](crate::Exif::parse) records the note's absolute source offset
//! ([`Exif::maker_note_offset`](crate::Exif::maker_note_offset)), and the writer **pins** the
//! note's byte range at that exact position on a rewrite — so TIFF-header-absolute internal
//! offsets a vendor encodes (common for Canon and many Nikon) stay valid even when edits shift
//! the surrounding directories (issue #263). Only when the new layout makes the pin
//! unsatisfiable (the directory region grows past the note's old position) does the writer fall
//! back to relocating the block, in which case such vendor-internal offsets may go stale.

/// A decoded-only-as-far-as-the-vendor MakerNote block: its detected vendor and its raw bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MakerNote {
    /// The vendor whose dialect the block follows (detected from `Make`; [`MakerNoteVendor::Unknown`]
    /// if unrecognised).
    pub vendor: MakerNoteVendor,
    /// The opaque, verbatim block bytes.
    pub bytes: Vec<u8>,
}

/// A MakerNote vendor dialect.
///
/// `#[non_exhaustive]`: more vendors — and, later, per-vendor decoding — can be added without a
/// breaking change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum MakerNoteVendor {
    /// Canon.
    Canon,
    /// Nikon.
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
    /// An unrecognised or absent vendor.
    Unknown,
}

impl MakerNoteVendor {
    /// Detects the vendor from the `Make` tag.
    ///
    /// A heuristic match on the manufacturer string — enough to route to a future per-vendor
    /// decoder, without inspecting the block's own (vendor-specific) signature.
    #[must_use]
    pub fn detect(make: Option<&str>) -> MakerNoteVendor {
        let Some(make) = make else {
            return MakerNoteVendor::Unknown;
        };
        let make = make.to_ascii_uppercase();
        let has = |needle: &str| make.contains(needle);
        if has("CANON") {
            MakerNoteVendor::Canon
        } else if has("NIKON") {
            MakerNoteVendor::Nikon
        } else if has("SONY") {
            MakerNoteVendor::Sony
        } else if has("FUJI") {
            MakerNoteVendor::Fujifilm
        } else if has("OLYMPUS") || has("OM DIGITAL") {
            MakerNoteVendor::Olympus
        } else if has("PANASONIC") || has("LUMIX") {
            MakerNoteVendor::Panasonic
        } else if has("APPLE") {
            MakerNoteVendor::Apple
        } else {
            MakerNoteVendor::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_vendors_case_insensitively() {
        assert_eq!(
            MakerNoteVendor::detect(Some("Canon")),
            MakerNoteVendor::Canon
        );
        assert_eq!(
            MakerNoteVendor::detect(Some("NIKON CORPORATION")),
            MakerNoteVendor::Nikon
        );
        assert_eq!(MakerNoteVendor::detect(Some("SONY")), MakerNoteVendor::Sony);
        assert_eq!(
            MakerNoteVendor::detect(Some("FUJIFILM")),
            MakerNoteVendor::Fujifilm
        );
        assert_eq!(
            MakerNoteVendor::detect(Some("OM Digital Solutions")),
            MakerNoteVendor::Olympus
        );
        assert_eq!(
            MakerNoteVendor::detect(Some("Panasonic")),
            MakerNoteVendor::Panasonic
        );
        assert_eq!(
            MakerNoteVendor::detect(Some("Apple")),
            MakerNoteVendor::Apple
        );
        assert_eq!(
            MakerNoteVendor::detect(Some("Hasselblad")),
            MakerNoteVendor::Unknown
        );
        assert_eq!(MakerNoteVendor::detect(None), MakerNoteVendor::Unknown);
    }
}
