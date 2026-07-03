//! The facade's error type.

use gamut_exif::ExifError;
use gamut_icc::IccError;
use gamut_iptc::IptcError;
use gamut_xmp::XmpError;

/// An error from extracting a unified [`Metadata`](crate::Metadata) or embedding it back to blocks.
///
/// Each variant names the standard whose parser or serializer failed and forwards that carrier's
/// own error enum ([`ExifError`], [`XmpError`], [`IccError`], [`IptcError`]), preserving its detail.
/// The variants are kept distinct so the failing carrier is always identifiable.
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
    Icc(#[from] IccError),
    /// Decoding the legacy IPTC-IIM carrier, or projecting IPTC back to it, failed.
    #[error("IPTC: {0}")]
    Iptc(#[from] IptcError),
}

/// A [`Result`](core::result::Result) whose error is [`MetadataError`].
pub type Result<T> = core::result::Result<T, MetadataError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_names_the_failing_carrier() {
        assert!(
            MetadataError::Icc(IccError::Malformed("bad"))
                .to_string()
                .starts_with("ICC:")
        );
        assert!(
            MetadataError::Iptc(IptcError::Unsupported("charset"))
                .to_string()
                .starts_with("IPTC:")
        );
    }

    #[test]
    fn leaf_errors_convert_into_named_variants() {
        let exif: MetadataError = ExifError::BadByteOrder.into();
        assert!(matches!(exif, MetadataError::Exif(_)));
        assert!(exif.to_string().starts_with("EXIF:"));

        let xmp: MetadataError = XmpError::MissingRdf.into();
        assert!(matches!(xmp, MetadataError::Xmp(_)));
        assert!(xmp.to_string().starts_with("XMP:"));

        let icc: MetadataError = IccError::Malformed("bad").into();
        assert!(matches!(icc, MetadataError::Icc(_)));
        assert!(icc.to_string().starts_with("ICC:"));

        let iptc: MetadataError = IptcError::Malformed("bad").into();
        assert!(matches!(iptc, MetadataError::Iptc(_)));
        assert!(iptc.to_string().starts_with("IPTC:"));
    }
}
