//! XML namespaces and the well-known XMP schemas.

/// The RDF namespace URI (`rdf:` prefix), the syntax XMP serializes into (Part 1 §6.2).
pub const RDF_NAMESPACE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

/// The XML namespace URI (`xml:` prefix), home of the `xml:lang` qualifier (Part 1 §6.2).
pub const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";

/// The `x:xmpmeta` wrapper namespace URI (`x:` prefix), the optional outer element (Part 1 §7.3.3).
pub const XMPMETA_NAMESPACE: &str = "adobe:ns:meta/";

/// An XML namespace: the URI that scopes property names, and its conventional prefix.
pub struct Namespace {
    /// The namespace URI (the canonical identity of the schema).
    pub uri: String,
    /// The conventional prefix used in serialization (e.g. `dc`, `xmp`).
    pub prefix: String,
}

/// The standard XMP schemas (Adobe XMP Part 2). Representative subset; the registry is filled in
/// during implementation. Each maps to a fixed namespace URI and conventional prefix.
pub enum WellKnownNs {
    /// `dc` — Dublin Core (title, creator, description, subject, rights, …).
    DublinCore,
    /// `xmp` — the basic XMP schema (CreateDate, ModifyDate, CreatorTool, …).
    Xmp,
    /// `xmpRights` — rights-management schema.
    XmpRights,
    /// `xmpMM` — media-management schema (DocumentID, InstanceID, history).
    XmpMediaManagement,
    /// `photoshop` — Adobe Photoshop schema.
    Photoshop,
    /// `exif` — EXIF tags mirrored into XMP.
    Exif,
    /// `tiff` — TIFF/EXIF image tags mirrored into XMP.
    Tiff,
    /// `Iptc4xmpCore` — IPTC Photo Metadata Core.
    Iptc4XmpCore,
    /// `Iptc4xmpExt` — IPTC Photo Metadata Extension.
    Iptc4XmpExt,
    /// `crs` — Camera Raw settings.
    CameraRaw,
}
