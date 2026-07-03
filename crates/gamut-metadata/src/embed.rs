//! Serializing a unified model back to metadata blocks.

use gamut_exif::Exif;
use gamut_icc::IccProfile;
use gamut_iptc::{IimCharset, IptcWriter};
use gamut_xmp::XmpMeta;

use crate::error::{MetadataError, Result};
use crate::metadata::Metadata;

/// The per-carrier byte blocks produced by [`MetadataEmbedder::embed`] — the owned inverse of the
/// [`MetadataBlock`](crate::MetadataBlock)s an [extractor](crate::MetadataExtractor) consumes.
///
/// Each field is `Some` when that carrier was produced; a container crate writes each present block
/// as its chunk / item / segment. IPTC-Core travels inside [`xmp`](Self::xmp) (the XMP packet);
/// [`iptc_iim`](Self::iptc_iim) is the optional legacy binary form, emitted only when requested via
/// [`MetadataEmbedder::emit_iptc_iim`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
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
}

impl Default for MetadataEmbedder {
    fn default() -> Self {
        Self {
            emit_iptc_iim: false,
            iim_charset: IimCharset::Utf8,
        }
    }
}

impl MetadataEmbedder {
    /// Creates an embedder with the default options (no legacy IIM block; UTF-8 IIM charset).
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

    /// Serializes the model to its per-carrier blocks.
    ///
    /// # Errors
    ///
    /// Returns a [`MetadataError`] naming the carrier whose serialization failed — [`MetadataError::Icc`]
    /// for an ICC profile that violates an invariant, or [`MetadataError::Iptc`] when
    /// [`emit_iptc_iim`](Self::emit_iptc_iim) is set and an IPTC value cannot be expressed in the
    /// chosen IIM charset.
    pub fn embed(&self, meta: &Metadata) -> Result<EncodedMetadata> {
        let exif = meta.exif.as_ref().map(Exif::to_bytes);
        let xmp = meta.xmp.as_ref().map(XmpMeta::to_packet);
        let icc = meta
            .icc
            .as_ref()
            .map(IccProfile::to_bytes)
            .transpose()
            .map_err(MetadataError::Icc)?;
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
        let block = IptcWriter::new()
            .charset(self.iim_charset)
            .write_iim(&pm)
            .map_err(MetadataError::Iptc)?;
        if block.datasets.is_empty() {
            return Ok(None);
        }
        block.encode().map(Some).map_err(MetadataError::Iptc)
    }
}
