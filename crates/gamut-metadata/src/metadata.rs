//! The unified metadata model.

use gamut_exif::Exif;
use gamut_icc::IccProfile;
use gamut_iptc::PhotoMetadata;
use gamut_xmp::XmpMeta;

use crate::embed::{EncodedMetadata, MetadataEmbedder};
use crate::error::Result;
use crate::extract::MetadataExtractor;
use crate::source::MetadataBlock;

/// All of an image's metadata, unified across the carriers a container holds.
///
/// There is exactly **one field per genuinely distinct serialization** a still-image container
/// carries:
///
/// - [`exif`](Self::exif) — the EXIF blob (a TIFF/IFD binary stream, e.g. a JPEG `APP1` /
///   WebP `EXIF` / AVIF `Exif` payload);
/// - [`xmp`](Self::xmp) — the XMP packet (the RDF/XML property graph);
/// - [`icc`](Self::icc) — the embedded ICC colour profile.
///
/// Each field is `Some` only when that carrier was present.
///
/// # Why there is no `iptc` field
///
/// IPTC Photo Metadata (Core + Extension) **is XMP** — properties in the `dc:`/`photoshop:`/
/// `xmpRights:`/`Iptc4xmp*` namespaces — not an independent serialization. It therefore lives inside
/// [`xmp`](Self::xmp), the single source of truth; storing it a second time would duplicate the same
/// data. The one genuinely separate IPTC carrier is the *legacy binary IIM* block (JPEG `APP13`
/// `8BIM 0x0404`, TIFF/DNG tag 33723); the [extractor](crate::MetadataExtractor) reconciles it
/// *into* `xmp`, and the [embedder](crate::MetadataEmbedder) projects it back out only on request.
/// Read the IPTC view with [`iptc`](Self::iptc) — a typed lens over `xmp` that stores nothing.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Metadata {
    /// EXIF metadata (camera/capture parameters, GPS, thumbnail), if present.
    pub exif: Option<Exif>,
    /// XMP metadata — the RDF/XML property graph, which also carries IPTC Photo Metadata — if
    /// present.
    pub xmp: Option<XmpMeta>,
    /// The embedded ICC colour profile, if present.
    pub icc: Option<IccProfile>,
}

impl Metadata {
    /// Extracts a unified model from already-located container metadata blocks, using default
    /// options. A convenience for [`MetadataExtractor::new().extract(blocks)`](MetadataExtractor::extract);
    /// use [`MetadataExtractor`] directly to choose an IPTC [`ConflictPolicy`](crate::ConflictPolicy).
    ///
    /// # Errors
    ///
    /// As [`MetadataExtractor::extract`].
    pub fn from_blocks(blocks: &[MetadataBlock<'_>]) -> Result<Self> {
        MetadataExtractor::new().extract(blocks)
    }

    /// Serializes this model back to per-carrier byte blocks, using default options. A convenience for
    /// [`MetadataEmbedder::new().embed(self)`](MetadataEmbedder::embed); use [`MetadataEmbedder`]
    /// directly to also emit the legacy IPTC-IIM block.
    ///
    /// # Errors
    ///
    /// As [`MetadataEmbedder::embed`].
    pub fn encode(&self) -> Result<EncodedMetadata> {
        MetadataEmbedder::new().embed(self)
    }

    /// The IPTC Photo Metadata view over [`xmp`](Self::xmp), or `None` when no XMP is present or the
    /// XMP carries no IPTC-namespace properties.
    ///
    /// This is a *computed lens* — the IPTC-relevant subset of the XMP graph, produced on demand via
    /// [`PhotoMetadata::from_xmp`] — not stored state. Mutating the returned value does **not** change
    /// `self`; to edit IPTC, edit [`xmp`](Self::xmp) (IPTC properties live there).
    #[must_use]
    pub fn iptc(&self) -> Option<PhotoMetadata> {
        let pm = PhotoMetadata::from_xmp(self.xmp.as_ref()?);
        (!pm.xmp.properties.is_empty()).then_some(pm)
    }

    /// Whether no metadata carrier is present (every field is `None`).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.exif.is_none() && self.xmp.is_none() && self.icc.is_none()
    }
}
