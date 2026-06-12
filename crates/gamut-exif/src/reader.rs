//! The EXIF reader.

/// Reader for an EXIF blob.
///
/// Will validate the `Exif\0\0` marker, parse the TIFF stream via [`gamut_ifd`], follow the
/// Exif/GPS/Interop pointer tags, and lift raw IFD values into typed [`crate::ExifValue`]s — and,
/// later, decode the `MakerNote`. Implementation pending (see issue #34).
pub struct ExifReader;
