//! Serializing a unified model back to metadata blocks.

use gamut_exif::Exif;
use gamut_icc::IccProfile;
use gamut_iptc::{IimCharset, IptcWriter};
use gamut_xmp::XmpMeta;

use crate::error::{MetadataError, Result};
use crate::metadata::Metadata;

/// What [`MetadataEmbedder::embed`] does with a model's
/// [extensions](crate::Metadata::extensions).
///
/// Extensions hold data no carrier can express, so there is nothing to serialize them into. This
/// chooses between losing them quietly and being told.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
#[repr(u8)]
pub enum ExtensionPolicy {
    /// Drop them. Extensions are process-local; the emitted blocks are unaffected. **Default.**
    #[default]
    Drop = 0,
    /// Fail with [`MetadataError::UnembeddableExtension`] when the model carries any extension —
    /// for a caller that must not silently lose data on the way to a file.
    Reject = 1,
}

/// The per-carrier byte blocks produced by [`MetadataEmbedder::embed`] — the owned inverse of the
/// [`MetadataBlock`](crate::MetadataBlock)s an [extractor](crate::MetadataExtractor) consumes.
///
/// Each field is `Some` when that carrier was produced; a container crate writes each present block
/// as its chunk / item / segment. IPTC-Core travels inside [`xmp`](Self::xmp) (the XMP packet);
/// [`iptc_iim`](Self::iptc_iim) is the optional legacy binary form, emitted only when requested via
/// [`MetadataEmbedder::emit_iptc_iim`].
///
/// A model's [extensions](Metadata::extensions) produce no block — they have no carrier — so
/// nothing here corresponds to them.
///
/// Marked `#[non_exhaustive]`, so a later carrier can add a field without a breaking change;
/// build one with [`EncodedMetadata::default`] plus field assignment rather than a struct literal.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
#[must_use]
pub struct EncodedMetadata {
    /// The EXIF blob (`Exif\0\0` + TIFF stream), if [`Metadata::exif`] was present.
    pub exif: Option<Vec<u8>>,
    /// The XMP packet (RDF/XML), which also carries IPTC-Core, if [`Metadata::xmp`] was present.
    pub xmp: Option<Vec<u8>>,
    /// The ICC profile blob, if [`Metadata::icc`] was present.
    pub icc: Option<Vec<u8>>,
    /// The legacy IPTC-IIM dataset stream (the payload a container wraps in a Photoshop `0x0404`
    /// resource), emitted only when [`MetadataEmbedder::emit_iptc_iim`] is set and the model carries
    /// IIM-expressible IPTC data.
    pub iptc_iim: Option<Vec<u8>>,
}

/// Serializes a [`Metadata`] back into per-carrier byte blocks for a container to embed — the inverse
/// of [`MetadataExtractor`](crate::MetadataExtractor).
///
/// EXIF, XMP, and ICC each serialize their present field. IPTC-Core is already inside the XMP packet,
/// so it needs no separate work; the **legacy** binary IIM block is opt-in ([`emit_iptc_iim`]) — it
/// cannot represent the richer IPTC Extension data XMP can, so the default keeps embedding lossless.
///
/// ```no_run
/// use gamut_metadata::{Metadata, MetadataEmbedder};
///
/// # fn demo(meta: &Metadata) -> Result<(), gamut_metadata::MetadataError> {
/// let blocks = MetadataEmbedder::new().embed(meta)?;
/// if let Some(xmp) = &blocks.xmp {
///     // container.write_xmp_chunk(xmp);
///     let _ = xmp;
/// }
/// # Ok(())
/// # }
/// ```
///
/// [`emit_iptc_iim`]: MetadataEmbedder::emit_iptc_iim
#[derive(Debug, Clone, Copy)]
pub struct MetadataEmbedder {
    emit_iptc_iim: bool,
    iim_charset: IimCharset,
    extension_policy: ExtensionPolicy,
}

impl Default for MetadataEmbedder {
    fn default() -> Self {
        Self {
            emit_iptc_iim: false,
            iim_charset: IimCharset::Utf8,
            extension_policy: ExtensionPolicy::Drop,
        }
    }
}

impl MetadataEmbedder {
    /// Creates an embedder with the default options (no legacy IIM block; UTF-8 IIM charset;
    /// extensions dropped).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether to also emit the legacy binary [`EncodedMetadata::iptc_iim`] block, and returns the
    /// embedder. Off by default — XMP already carries IPTC losslessly; the IIM projection is
    /// best-effort and for legacy-consumer compatibility only.
    #[must_use]
    pub fn emit_iptc_iim(mut self, emit: bool) -> Self {
        self.emit_iptc_iim = emit;
        self
    }

    /// Sets the charset used to encode the legacy IIM text, and returns the embedder. Defaults to
    /// [`IimCharset::Utf8`]; only relevant when [`emit_iptc_iim`](Self::emit_iptc_iim) is set.
    #[must_use]
    pub fn iim_charset(mut self, charset: IimCharset) -> Self {
        self.iim_charset = charset;
        self
    }

    /// Sets what to do with the model's [extensions](Metadata::extensions), and returns the
    /// embedder. Defaults to [`ExtensionPolicy::Drop`].
    #[must_use]
    pub fn extension_policy(mut self, policy: ExtensionPolicy) -> Self {
        self.extension_policy = policy;
        self
    }

    /// Serializes the model to its per-carrier blocks.
    ///
    /// The model's [extensions](Metadata::extensions) are not carriers and produce no block; they
    /// are dropped, or rejected, per [`extension_policy`](Self::extension_policy).
    ///
    /// # Errors
    ///
    /// Returns a [`MetadataError`](crate::MetadataError) naming the carrier whose serialization
    /// failed — [`MetadataError::Exif`](crate::MetadataError::Exif) for an EXIF model not
    /// representable in classic-TIFF widths,
    /// [`MetadataError::Icc`](crate::MetadataError::Icc) for an ICC profile that violates
    /// an invariant, or [`MetadataError::Iptc`](crate::MetadataError::Iptc) when
    /// [`emit_iptc_iim`](Self::emit_iptc_iim) is set and an IPTC value cannot be expressed in the
    /// chosen IIM charset.
    ///
    /// Also returns [`MetadataError::UnembeddableExtension`](crate::MetadataError::UnembeddableExtension),
    /// naming the first extension, when [`extension_policy`](Self::extension_policy) is
    /// [`ExtensionPolicy::Reject`] and the model carries one.
    pub fn embed(&self, meta: &Metadata) -> Result<EncodedMetadata> {
        // Refuse before serializing anything, so a rejected model produces no partial work.
        if self.extension_policy == ExtensionPolicy::Reject
            && let Some(ext) = meta.extensions.first()
        {
            return Err(MetadataError::UnembeddableExtension {
                namespace: ext.namespace.clone(),
                key: ext.key.clone(),
            });
        }

        let exif = meta.exif.as_ref().map(Exif::to_bytes).transpose()?;
        let xmp = meta.xmp.as_ref().map(XmpMeta::to_packet);
        let icc = meta.icc.as_ref().map(IccProfile::to_bytes).transpose()?;
        let iptc_iim = if self.emit_iptc_iim {
            self.encode_iim(meta)?
        } else {
            None
        };
        Ok(EncodedMetadata {
            exif,
            xmp,
            icc,
            iptc_iim,
        })
    }

    /// Projects the model's IPTC view to a raw IIM dataset stream, or `None` when there is no
    /// IIM-expressible IPTC data.
    fn encode_iim(&self, meta: &Metadata) -> Result<Option<Vec<u8>>> {
        let Some(pm) = meta.iptc() else {
            return Ok(None);
        };
        let block = IptcWriter::new().charset(self.iim_charset).write_iim(&pm)?;
        if block.datasets.is_empty() {
            return Ok(None);
        }
        Ok(Some(block.encode()?))
    }
}
