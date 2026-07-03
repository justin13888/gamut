//! The directories EXIF tags live in.
//!
//! The [`ExifTag`] catalogue that names individual tags is layered on top (see the crate root);
//! this module holds the small, stable [`IfdKind`] that classifies which directory a tag belongs
//! to. EXIF spreads its tags across several IFDs reached from the 0th IFD through pointer tags
//! (Exif 3.0 §4.6).

/// Which IFD a tag belongs to.
///
/// The same 16-bit tag number can mean different things in different directories (e.g. `0x0001` is
/// `GPSLatitudeRef` in [`IfdKind::Gps`] but `InteroperabilityIndex` in [`IfdKind::Interop`]), so a
/// tag is only fully identified by the pair (`IfdKind`, id).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum IfdKind {
    /// The 0th IFD — primary-image / TIFF tags (Make, Model, Orientation, resolution, …).
    Image,
    /// The Exif sub-IFD — capture parameters (exposure, aperture, ISO, lens, …).
    Exif,
    /// The GPS sub-IFD — positioning data.
    Gps,
    /// The Interoperability sub-IFD — interoperability identification.
    Interop,
    /// The 1st IFD — the embedded thumbnail's tags.
    Thumbnail,
}
