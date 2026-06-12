//! XML namespaces and the well-known XMP schemas.

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
