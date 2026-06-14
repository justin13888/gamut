//! XML namespaces and the well-known XMP schemas.

/// The RDF namespace URI (`rdf:` prefix), the syntax XMP serializes into (Part 1 §6.2).
pub const RDF_NAMESPACE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

/// The XML namespace URI (`xml:` prefix), home of the `xml:lang` qualifier (Part 1 §6.2).
pub const XML_NAMESPACE: &str = "http://www.w3.org/XML/1998/namespace";

/// The `x:xmpmeta` wrapper namespace URI (`x:` prefix), the optional outer element (Part 1 §7.3.3).
pub const XMPMETA_NAMESPACE: &str = "adobe:ns:meta/";

/// An XML namespace: the URI that scopes property names, and its conventional prefix.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Namespace {
    /// The namespace URI (the canonical identity of the schema).
    pub uri: String,
    /// The conventional prefix used in serialization (e.g. `dc`, `xmp`).
    pub prefix: String,
}

impl Namespace {
    /// Creates a namespace from a URI and conventional prefix.
    #[must_use]
    pub fn new(uri: impl Into<String>, prefix: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            prefix: prefix.into(),
        }
    }

    /// The namespace for a standard schema, with its conventional prefix.
    #[must_use]
    pub fn well_known(ns: WellKnownNs) -> Self {
        Self::new(ns.uri(), ns.prefix())
    }
}

/// The standard XMP schemas (Adobe XMP Parts 1–2). Each maps to a fixed namespace URI and a
/// conventional prefix via [`WellKnownNs::uri`] / [`WellKnownNs::prefix`]; [`WellKnownNs::from_uri`]
/// recovers the schema from a URI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
    /// `xmpidq` — the qualifier schema for `xmpMM` identifiers (Part 1 §8.7).
    XmpIdentifier,
    /// `xmpBJ` — the Basic Job Ticket schema (Part 2 §2.3).
    XmpJobTicket,
    /// `xmpTPg` — the Paged-Text schema (Part 2 §2.4).
    XmpPagedText,
    /// `xmpDM` — the Dynamic Media schema (Part 2 §2.5).
    XmpDynamicMedia,
    /// `pdf` — the Adobe PDF schema (Part 2 §3.1).
    Pdf,
    /// `stDim` — the Dimensions structure type (used by, e.g., `xmpTPg:MaxPageSize`).
    Dimensions,
    /// `stRef` — the ResourceRef structure type (used by `xmpMM` references).
    ResourceRef,
}

impl WellKnownNs {
    /// Every standard schema, for iteration and lookup.
    pub const ALL: &'static [WellKnownNs] = &[
        WellKnownNs::DublinCore,
        WellKnownNs::Xmp,
        WellKnownNs::XmpRights,
        WellKnownNs::XmpMediaManagement,
        WellKnownNs::Photoshop,
        WellKnownNs::Exif,
        WellKnownNs::Tiff,
        WellKnownNs::Iptc4XmpCore,
        WellKnownNs::Iptc4XmpExt,
        WellKnownNs::CameraRaw,
        WellKnownNs::XmpIdentifier,
        WellKnownNs::XmpJobTicket,
        WellKnownNs::XmpPagedText,
        WellKnownNs::XmpDynamicMedia,
        WellKnownNs::Pdf,
        WellKnownNs::Dimensions,
        WellKnownNs::ResourceRef,
    ];

    /// The schema's namespace URI — its canonical identity.
    ///
    /// `dc` is Dublin Core (Part 1 §8.3); the `xmp*` schemas are Part 1 §8.4–8.6; `photoshop`,
    /// `crs` are Part 2 §3.2–3.3. `exif`/`tiff` mirror the EXIF tags into XMP (Part 2 §3.4, defined
    /// by CIPA DC-010); `Iptc4xmpCore`/`Iptc4xmpExt` are the IPTC Photo Metadata schemas.
    #[must_use]
    pub const fn uri(self) -> &'static str {
        match self {
            WellKnownNs::DublinCore => "http://purl.org/dc/elements/1.1/",
            WellKnownNs::Xmp => "http://ns.adobe.com/xap/1.0/",
            WellKnownNs::XmpRights => "http://ns.adobe.com/xap/1.0/rights/",
            WellKnownNs::XmpMediaManagement => "http://ns.adobe.com/xap/1.0/mm/",
            WellKnownNs::Photoshop => "http://ns.adobe.com/photoshop/1.0/",
            WellKnownNs::Exif => "http://ns.adobe.com/exif/1.0/",
            WellKnownNs::Tiff => "http://ns.adobe.com/tiff/1.0/",
            WellKnownNs::Iptc4XmpCore => "http://iptc.org/std/Iptc4xmpCore/1.0/xmlns/",
            WellKnownNs::Iptc4XmpExt => "http://iptc.org/std/Iptc4xmpExt/2008-02-29/",
            WellKnownNs::CameraRaw => "http://ns.adobe.com/camera-raw-settings/1.0/",
            WellKnownNs::XmpIdentifier => "http://ns.adobe.com/xmp/Identifier/qual/1.0/",
            WellKnownNs::XmpJobTicket => "http://ns.adobe.com/xap/1.0/bj/",
            WellKnownNs::XmpPagedText => "http://ns.adobe.com/xap/1.0/t/pg/",
            WellKnownNs::XmpDynamicMedia => "http://ns.adobe.com/xmp/1.0/DynamicMedia/",
            WellKnownNs::Pdf => "http://ns.adobe.com/pdf/1.3/",
            WellKnownNs::Dimensions => "http://ns.adobe.com/xap/1.0/sType/Dimensions#",
            WellKnownNs::ResourceRef => "http://ns.adobe.com/xap/1.0/sType/ResourceRef#",
        }
    }

    /// The schema's conventional prefix (e.g. `dc`, `xmp`, `Iptc4xmpCore`). The prefix is only a
    /// serialization convenience — the URI is the identity (Part 1 §6.2).
    #[must_use]
    pub const fn prefix(self) -> &'static str {
        match self {
            WellKnownNs::DublinCore => "dc",
            WellKnownNs::Xmp => "xmp",
            WellKnownNs::XmpRights => "xmpRights",
            WellKnownNs::XmpMediaManagement => "xmpMM",
            WellKnownNs::Photoshop => "photoshop",
            WellKnownNs::Exif => "exif",
            WellKnownNs::Tiff => "tiff",
            WellKnownNs::Iptc4XmpCore => "Iptc4xmpCore",
            WellKnownNs::Iptc4XmpExt => "Iptc4xmpExt",
            WellKnownNs::CameraRaw => "crs",
            WellKnownNs::XmpIdentifier => "xmpidq",
            WellKnownNs::XmpJobTicket => "xmpBJ",
            WellKnownNs::XmpPagedText => "xmpTPg",
            WellKnownNs::XmpDynamicMedia => "xmpDM",
            WellKnownNs::Pdf => "pdf",
            WellKnownNs::Dimensions => "stDim",
            WellKnownNs::ResourceRef => "stRef",
        }
    }

    /// The schema whose URI is exactly `uri`, if any.
    #[must_use]
    pub fn from_uri(uri: &str) -> Option<WellKnownNs> {
        WellKnownNs::ALL.iter().copied().find(|ns| ns.uri() == uri)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_and_prefix_are_exact_and_round_trip() {
        // Exact strings — a typo in any URI/prefix is a survived mutant otherwise.
        assert_eq!(
            WellKnownNs::DublinCore.uri(),
            "http://purl.org/dc/elements/1.1/"
        );
        assert_eq!(WellKnownNs::DublinCore.prefix(), "dc");
        assert_eq!(WellKnownNs::Xmp.uri(), "http://ns.adobe.com/xap/1.0/");
        assert_eq!(WellKnownNs::Iptc4XmpCore.prefix(), "Iptc4xmpCore");
        assert_eq!(
            WellKnownNs::XmpDynamicMedia.uri(),
            "http://ns.adobe.com/xmp/1.0/DynamicMedia/"
        );
        assert_eq!(WellKnownNs::Dimensions.prefix(), "stDim");
        assert_eq!(WellKnownNs::Pdf.uri(), "http://ns.adobe.com/pdf/1.3/");

        for &ns in WellKnownNs::ALL {
            assert_eq!(WellKnownNs::from_uri(ns.uri()), Some(ns));
            assert!(!ns.prefix().is_empty());
        }
    }

    #[test]
    fn from_uri_rejects_unknown() {
        assert_eq!(WellKnownNs::from_uri("http://example.com/ns/"), None);
    }

    #[test]
    fn all_uris_and_prefixes_are_unique() {
        for (i, &a) in WellKnownNs::ALL.iter().enumerate() {
            for &b in &WellKnownNs::ALL[i + 1..] {
                assert_ne!(a.uri(), b.uri(), "duplicate URI for {a:?}/{b:?}");
                assert_ne!(a.prefix(), b.prefix(), "duplicate prefix for {a:?}/{b:?}");
            }
        }
    }

    #[test]
    fn namespace_well_known_carries_uri_and_prefix() {
        let ns = Namespace::well_known(WellKnownNs::Photoshop);
        assert_eq!(ns.uri, "http://ns.adobe.com/photoshop/1.0/");
        assert_eq!(ns.prefix, "photoshop");
    }
}
