//! The unified metadata model.

use gamut_exif::Exif;
use gamut_icc::IccProfile;
use gamut_iptc::PhotoMetadata;
use gamut_xmp::XmpMeta;

/// All of an image's metadata, unified across the four standards.
///
/// Each field is present only if that kind of metadata was found. The same datum can legitimately
/// appear in more than one (e.g. the creation date in both EXIF and XMP); cross-format
/// reconciliation — harmonising those into a single coherent view, exiftool-style — is a later
/// implementation phase, not part of this model's shape.
pub struct Metadata {
    /// EXIF metadata (camera/capture parameters, GPS, thumbnail), if present.
    pub exif: Option<Exif>,
    /// XMP metadata (the RDF/XML property graph), if present.
    pub xmp: Option<XmpMeta>,
    /// The embedded ICC color profile, if present.
    pub icc: Option<IccProfile>,
    /// IPTC photo metadata (reconciled from IIM and/or IPTC-Core XMP), if present.
    pub iptc: Option<PhotoMetadata>,
}
