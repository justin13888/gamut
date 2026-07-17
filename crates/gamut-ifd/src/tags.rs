//! Well-known **structural** pointer tags.
//!
//! This crate deliberately owns no tag *semantics* — what `ImageWidth` or `Compression` mean is
//! codec business. These tags are the one principled exception, because they are *structure*:
//! their values locate other directories (or, for [`MAKER_NOTE`], a byte blob vendors fill with
//! a rebased mini-IFD), i.e. they name the directory graph itself — the graph
//! [`read_tree`](crate::read_tree) / [`IfdReader::read_tree`](crate::IfdReader::read_tree)
//! reconstruct and [`write`](crate::write) flattens. Before this module, every consumer
//! (gamut-tiff, gamut-dng, gamut-exif) duplicated them.

/// `SubIFDs` (330, 0x014A) — offsets of child image directories (TIFF/EP and DNG raw,
/// reduced-resolution, and mask subfiles).
pub const SUB_IFDS: u16 = 330;

/// `ExifIFD` (34665, 0x8769) — the offset of the Exif private sub-IFD (Exif 3.0 §4.6.3).
pub const EXIF_IFD: u16 = 34665;

/// `GPSInfo` (34853, 0x8825) — the offset of the GPS private sub-IFD.
pub const GPS_INFO: u16 = 34853;

/// `MakerNote` (37500, 0x927C) — an `UNDEFINED` vendor blob, usually a mini-IFD whose internal
/// offsets are relative to the note start or the enclosing TIFF header.
///
/// **Not** a pointer tag for [`read_tree`](crate::read_tree): its value is bytes, not a `LONG`
/// offset array. Decode it by hand with
/// [`IfdReader::with_layout`](crate::IfdReader::with_layout) over a
/// [`Rebased`](crate::Rebased) source once the vendor's offset convention is known.
pub const MAKER_NOTE: u16 = 37500;

/// `InteroperabilityIFD` (40965, 0xA005) — the offset of the Exif Interoperability sub-IFD
/// (reached from the Exif sub-IFD, not the 0th IFD).
pub const INTEROPERABILITY_IFD: u16 = 40965;

/// The standard pointer tags a TIFF/EP-, DNG-, or EXIF-shaped consumer follows with
/// [`read_tree`](crate::read_tree): [`SUB_IFDS`], [`EXIF_IFD`], [`GPS_INFO`], and
/// [`INTEROPERABILITY_IFD`]. ([`MAKER_NOTE`] is deliberately absent — see its docs.)
pub const STANDARD_POINTER_TAGS: &[u16] = &[SUB_IFDS, EXIF_IFD, GPS_INFO, INTEROPERABILITY_IFD];

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the constants to the spec-assigned numbers (TIFF/EP, Exif 3.0) and the pointer set
    /// to exactly the followable four.
    #[test]
    fn structural_tags_match_the_specs() {
        assert_eq!(SUB_IFDS, 0x014A);
        assert_eq!(EXIF_IFD, 0x8769);
        assert_eq!(GPS_INFO, 0x8825);
        assert_eq!(MAKER_NOTE, 0x927C);
        assert_eq!(INTEROPERABILITY_IFD, 0xA005);
        assert_eq!(
            STANDARD_POINTER_TAGS,
            &[SUB_IFDS, EXIF_IFD, GPS_INFO, INTEROPERABILITY_IFD]
        );
    }
}
