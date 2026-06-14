//! The IPTC reader — the convenient entry point for decoding IPTC metadata.

use gamut_core::Result;

use crate::iim::IimBlock;
use crate::irb::PhotoshopIrb;

/// Reader for IPTC metadata.
///
/// For the legacy carrier, [`IptcReader::read_irb`] decodes the IIM datasets from a Photoshop
/// image-resource stream. To parse a bare IIM dataset stream (the `0x0404` resource payload, e.g.
/// `gamut_metadata`'s `IptcIim` block) use [`IimBlock::parse`] directly.
///
/// The modern IPTC Photo Metadata (Core/Extension) path, interpreting a parsed XMP graph, is added
/// in [`crate::photo_metadata`] (see issue #34).
#[derive(Debug, Clone, Copy, Default)]
pub struct IptcReader;

impl IptcReader {
    /// Creates a new reader.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Parses a Photoshop image-resource (`8BIM`) stream and decodes the legacy IPTC-IIM datasets
    /// from its `0x0404` resource.
    ///
    /// Returns `Ok(None)` when the stream parses but carries no `0x0404` resource.
    ///
    /// # Errors
    ///
    /// Returns [`gamut_core::Error::InvalidInput`] if the resource stream or the IIM datasets are
    /// malformed (see [`PhotoshopIrb::parse`] and [`IimBlock::parse`]).
    pub fn read_irb(&self, irb: &[u8]) -> Result<Option<IimBlock>> {
        PhotoshopIrb::parse(irb)?
            .iptc_iim()
            .map(IimBlock::parse)
            .transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_irb_propagates_resource_stream_errors() {
        // A stream that is not a valid 8BIM resource fails at the IRB layer.
        assert!(IptcReader::new().read_irb(b"not an irb").is_err());
    }

    #[test]
    fn read_irb_propagates_iim_errors() {
        // A valid resource whose 0x0404 payload is a malformed IIM stream fails at the IIM layer.
        let irb = PhotoshopIrb::with_iptc(vec![0x00, 0x01]).encode().unwrap();
        assert!(IptcReader::new().read_irb(&irb).is_err());
    }
}
