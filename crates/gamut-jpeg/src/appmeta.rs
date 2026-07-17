//! APPn metadata payload conventions: APP1 EXIF, APP1 XMP, and multi-segment APP2 ICC.
//!
//! T.81 defines only the `APPn` marker syntax (§B.2.4.6); the payload layouts come from the
//! metadata standards, pinned with the framing constants in
//! [`references/jpeg`](https://github.com/justin13888/gamut/tree/master/references/jpeg):
//!
//! - **APP1 EXIF** (Exif 3.0, CIPA DC-008 §4.7.2): the signature `"Exif\0\0"` followed by a TIFF
//!   stream. A single segment.
//! - **APP1 XMP** (XMP Part 3 §1.1.3): the null-terminated namespace URI
//!   `"http://ns.adobe.com/xap/1.0/"` followed by one XMP `xpacket` of at most 65502 bytes (the
//!   spec-stated StandardXMP cap). The ExtendedXMP continuation scheme (§1.1.3.1, signature
//!   `"http://ns.adobe.com/xmp/extension/\0"`) is deferred — see `STATUS.md`.
//! - **APP2 ICC** (ICC.1:2001-04 Annex B.4): the signature `"ICC_PROFILE\0"` followed by a 1-based
//!   chunk index byte and a total chunk-count byte, the profile split across up to 255 chunks of at
//!   most 65519 bytes. T.81 does not guarantee segment order, so reassembly is by index.

use gamut_core::{Error, Result};

/// APP1 EXIF signature (Exif 3.0 §4.7.2.3): `"Exif"` plus two NUL padding bytes.
pub(crate) const EXIF_SIG: &[u8] = b"Exif\0\0";

/// APP1 XMP signature (XMP Part 3 §1.1.3): the null-terminated `xap/1.0` namespace URI. Note the
/// ExtendedXMP signature uses the distinct `xmp/extension/` URI and never matches this prefix.
pub(crate) const XMP_SIG: &[u8] = b"http://ns.adobe.com/xap/1.0/\0";

/// APP2 ICC signature (ICC.1:2001-04 Annex B.4), followed by the chunk index and count bytes.
pub(crate) const ICC_SIG: &[u8] = b"ICC_PROFILE\0";

/// Returns the TIFF stream of an APP1 EXIF payload, or `None` if the signature does not match.
pub(crate) fn exif_payload(payload: &[u8]) -> Option<&[u8]> {
    payload.strip_prefix(EXIF_SIG)
}

/// Returns the `xpacket` of an APP1 XMP payload, or `None` if the signature does not match
/// (including ExtendedXMP segments, whose URI differs).
pub(crate) fn xmp_payload(payload: &[u8]) -> Option<&[u8]> {
    payload.strip_prefix(XMP_SIG)
}

/// Reassembles an ICC profile from its APP2 `ICC_PROFILE` chunks (ICC.1:2001-04 Annex B.4).
///
/// Chunks may arrive in any order; each carries a 1-based index and the shared total count, and
/// every chunk must agree on that count. Feed each APP2 payload to [`IccAssembler::add`] and take
/// the profile from [`IccAssembler::finish`].
#[derive(Debug, Default)]
pub(crate) struct IccAssembler {
    /// Chunk slots, allocated to the declared count when the first ICC chunk arrives; `None` slots
    /// are chunks not yet seen. Empty until then.
    chunks: Vec<Option<Vec<u8>>>,
}

impl IccAssembler {
    /// Records one APP2 payload. Payloads without the `ICC_PROFILE` signature are ignored (APP2 is
    /// also used by e.g. Exif Flashpix extensions).
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] on a truncated chunk header, a zero index or count, an index
    /// beyond the count, a count that disagrees with earlier chunks, or a repeated index.
    pub(crate) fn add(&mut self, payload: &[u8]) -> Result<()> {
        let Some(rest) = payload.strip_prefix(ICC_SIG) else {
            return Ok(());
        };
        let [index, count, data @ ..] = rest else {
            return Err(Error::InvalidInput("JPEG: truncated ICC_PROFILE chunk"));
        };
        let (index, count) = (usize::from(*index), usize::from(*count));
        if index == 0 || count == 0 || index > count {
            return Err(Error::InvalidInput(
                "JPEG: ICC_PROFILE chunk index out of range",
            ));
        }
        if self.chunks.is_empty() {
            self.chunks.resize(count, None);
        } else if self.chunks.len() != count {
            return Err(Error::InvalidInput(
                "JPEG: ICC_PROFILE chunk count mismatch",
            ));
        }
        let slot = &mut self.chunks[index - 1];
        if slot.is_some() {
            return Err(Error::InvalidInput("JPEG: duplicate ICC_PROFILE chunk"));
        }
        *slot = Some(data.to_vec());
        Ok(())
    }

    /// Concatenates the chunks in index order, or `None` if no ICC chunk was seen.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if any chunk of the declared count is missing.
    pub(crate) fn finish(self) -> Result<Option<Vec<u8>>> {
        if self.chunks.is_empty() {
            return Ok(None);
        }
        let mut profile = Vec::new();
        for chunk in self.chunks {
            let chunk = chunk.ok_or(Error::InvalidInput("JPEG: missing ICC_PROFILE chunk"))?;
            profile.extend_from_slice(&chunk);
        }
        Ok(Some(profile))
    }
}
