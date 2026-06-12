//! The EXIF tag dictionary and the directories tags live in.

/// Which IFD a tag belongs to. EXIF spreads its tags across several directories, reached from the
/// 0th IFD through pointer tags (Exif 3.0 §4.6).
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

/// An EXIF tag identifier. Representative subset spanning each directory; the full dictionary
/// (exiftool-class coverage) is filled in during implementation. Each maps to a 16-bit on-disk tag
/// number within its [`IfdKind`].
pub enum ExifTag {
    /// `0x0100` ImageWidth (0th/1st IFD).
    ImageWidth,
    /// `0x0101` ImageLength (0th/1st IFD).
    ImageLength,
    /// `0x010F` Make — camera manufacturer (0th IFD).
    Make,
    /// `0x0110` Model — camera model (0th IFD).
    Model,
    /// `0x0112` Orientation (0th IFD).
    Orientation,
    /// `0x0131` Software (0th IFD).
    Software,
    /// `0x0132` DateTime — file change date/time (0th IFD).
    DateTime,
    /// `0x8769` ExifIFDPointer — offset of the Exif sub-IFD (0th IFD).
    ExifIfdPointer,
    /// `0x8825` GPSInfoIFDPointer — offset of the GPS sub-IFD (0th IFD).
    GpsIfdPointer,
    /// `0x829A` ExposureTime (Exif IFD).
    ExposureTime,
    /// `0x829D` FNumber (Exif IFD).
    FNumber,
    /// `0x8827` PhotographicSensitivity / ISOSpeedRatings (Exif IFD).
    PhotographicSensitivity,
    /// `0x9003` DateTimeOriginal (Exif IFD).
    DateTimeOriginal,
    /// `0x920A` FocalLength (Exif IFD).
    FocalLength,
    /// `0xA434` LensModel (Exif IFD).
    LensModel,
    /// `0x927C` MakerNote — vendor-specific block (Exif IFD).
    MakerNote,
}
