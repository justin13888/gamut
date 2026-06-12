//! The modern IPTC Photo Metadata (Core + Extension), expressed over XMP.

use gamut_xmp::XmpProperty;

/// IPTC Photo Metadata (Core + Extension), the modern standard.
///
/// These properties are defined *as* XMP (in the `Iptc4xmpCore` / `Iptc4xmpExt` namespaces), so the
/// model is carried as [`gamut_xmp`] properties rather than a parallel type hierarchy. Typed
/// accessors for the well-known fields (creator, location, rights, licensing) are added during
/// implementation.
pub struct PhotoMetadata {
    /// The IPTC Core/Extension properties, as XMP properties in the IPTC namespaces.
    pub properties: Vec<XmpProperty>,
}
