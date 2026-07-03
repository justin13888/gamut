//! The facade's error type.

use gamut_exif::ExifError;
use gamut_xmp::XmpError;

/// An error from extracting a unified [`Metadata`](crate::Metadata) or embedding it back to blocks.
///
/// Each variant names the standard whose parser or serializer failed, preserving the underlying
/// error's detail. EXIF and XMP expose their own rich error enums (forwarded here); ICC and IPTC both
/// surface [`gamut_core::Error`], kept in distinct variants so the failing carrier is always
/// identifiable.
///
/// Marked `#[non_exhaustive]` so a new carrier can add a variant without a breaking change.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum MetadataError {
    /// Parsing or serializing the EXIF blob failed.
    #[error("EXIF: {0}")]
    Exif(#[from] ExifError),
    /// Parsing the XMP packet failed. (XMP serialization is infallible.)
    #[error("XMP: {0}")]
    Xmp(#[from] XmpError),
    /// Parsing or serializing the ICC profile failed.
    #[error("ICC: {0}")]
    Icc(gamut_core::Error),
    /// Decoding the legacy IPTC-IIM carrier, or projecting IPTC back to it, failed.
    #[error("IPTC: {0}")]
    Iptc(gamut_core::Error),
}

/// A [`Result`](core::result::Result) whose error is [`MetadataError`].
pub type Result<T> = core::result::Result<T, MetadataError>;
