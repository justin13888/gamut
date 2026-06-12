//! The EXIF writer.

/// Writer for an EXIF blob.
///
/// Will serialise an [`crate::Exif`] back to a valid `Exif\0\0` + TIFF stream via [`gamut_ifd`]'s
/// offset-patching writer, preserving the source byte order. The crate's **keystone** is a
/// round-trip (parse → write → parse) that re-emits a valid blob with the sub-IFD pointers and
/// thumbnail intact. Implementation pending (see issue #34).
pub struct ExifWriter;
