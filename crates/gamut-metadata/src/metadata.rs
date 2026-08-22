//! The unified metadata model.

use gamut_exif::{Exif, Value};
use gamut_icc::IccProfile;
use gamut_iptc::PhotoMetadata;
use gamut_xmp::XmpMeta;

use crate::embed::{EncodedMetadata, MetadataEmbedder};
use crate::error::Result;
use crate::extension::MetadataExtension;
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
///
/// # Extensions
///
/// [`extensions`](Self::extensions) is deliberately **not** a fourth carrier: it holds data no
/// carrier can express, so a downstream typed model survives `their model → Metadata → their
/// model` in full. It does not serialize — see [`MetadataExtension`] and the
/// [crate docs](crate#extensions-data-with-no-carrier).
///
/// # Construction
///
/// Marked `#[non_exhaustive]`, so build one with [`from_carriers`](Self::from_carriers), or with
/// [`Metadata::default`] followed by field assignment, rather than a struct literal.
#[derive(Debug, Clone, Default, PartialEq)]
#[non_exhaustive]
pub struct Metadata {
    /// EXIF metadata (camera/capture parameters, GPS, thumbnail), if present.
    pub exif: Option<Exif>,
    /// XMP metadata — the RDF/XML property graph, which also carries IPTC Photo Metadata — if
    /// present.
    pub xmp: Option<XmpMeta>,
    /// The embedded ICC colour profile, if present.
    pub icc: Option<IccProfile>,
    /// Data none of the carriers above models, in namespaces the caller owns.
    ///
    /// **Never serialized** — [`encode`](Self::encode) drops these (see
    /// [`ExtensionPolicy`](crate::ExtensionPolicy)) and extraction never produces them. Order is
    /// preserved; a `(namespace, key)` pair appears at most once when maintained through
    /// [`set_extension`](Self::set_extension).
    pub extensions: Vec<MetadataExtension>,
}

impl Metadata {
    /// Builds a model from the three carriers, with no [`extensions`](Self::extensions).
    ///
    /// The `#[non_exhaustive]` replacement for a `Metadata { exif, xmp, icc }` struct literal.
    #[must_use]
    pub fn from_carriers(
        exif: Option<Exif>,
        xmp: Option<XmpMeta>,
        icc: Option<IccProfile>,
    ) -> Self {
        Self {
            exif,
            xmp,
            icc,
            extensions: Vec::new(),
        }
    }

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

    /// The value bound to `key` in `namespace`, or `None` when the model carries no such
    /// [extension](Self::extensions).
    #[must_use]
    pub fn extension(&self, namespace: &str, key: &str) -> Option<&Value> {
        self.extensions
            .iter()
            .find(|e| e.namespace == namespace && e.key == key)
            .map(|e| &e.value)
    }

    /// Binds `key` in `namespace` to `value`, replacing the existing binding in place if there is
    /// one and appending otherwise — so a `(namespace, key)` pair never appears twice.
    pub fn set_extension(
        &mut self,
        namespace: impl Into<String>,
        key: impl Into<String>,
        value: Value,
    ) {
        let (namespace, key) = (namespace.into(), key.into());
        match self
            .extensions
            .iter_mut()
            .find(|e| e.namespace == namespace && e.key == key)
        {
            Some(existing) => existing.value = value,
            None => self
                .extensions
                .push(MetadataExtension::new(namespace, key, value)),
        }
    }

    /// Removes the binding for `key` in `namespace`, returning its value if there was one.
    pub fn remove_extension(&mut self, namespace: &str, key: &str) -> Option<Value> {
        let index = self
            .extensions
            .iter()
            .position(|e| e.namespace == namespace && e.key == key)?;
        Some(self.extensions.remove(index).value)
    }

    /// The [extensions](Self::extensions) in `namespace`, in insertion order.
    pub fn extensions_in<'a>(
        &'a self,
        namespace: &'a str,
    ) -> impl Iterator<Item = &'a MetadataExtension> {
        self.extensions
            .iter()
            .filter(move |e| e.namespace == namespace)
    }

    /// Whether the model holds nothing at all — no carrier and no
    /// [extension](Self::extensions).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.exif.is_none()
            && self.xmp.is_none()
            && self.icc.is_none()
            && self.extensions.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use gamut_xmp::WellKnownNs;

    use super::*;

    fn xmp_with(namespace: &str, name: &str, value: &str) -> XmpMeta {
        let mut xmp = XmpMeta::new();
        xmp.set_text(namespace, name, value);
        xmp
    }

    #[test]
    fn iptc_lens_reflects_only_iptc_namespaces() {
        // An IPTC namespace (photoshop:City) surfaces through the lens...
        let iptc = Metadata {
            xmp: Some(xmp_with(WellKnownNs::Photoshop.uri(), "City", "Oslo")),
            ..Default::default()
        };
        assert_eq!(iptc.iptc().unwrap().city(), Some("Oslo"));

        // ...a non-IPTC namespace (xmp:CreatorTool) does not.
        let non_iptc = Metadata {
            xmp: Some(xmp_with(WellKnownNs::Xmp.uri(), "CreatorTool", "gamut")),
            ..Default::default()
        };
        assert!(non_iptc.iptc().is_none());

        // No XMP at all → no IPTC view.
        assert!(Metadata::default().iptc().is_none());
    }

    #[test]
    fn is_empty_tracks_field_presence() {
        assert!(Metadata::default().is_empty());
        assert!(
            !Metadata {
                xmp: Some(XmpMeta::new()),
                ..Default::default()
            }
            .is_empty()
        );
    }
}
