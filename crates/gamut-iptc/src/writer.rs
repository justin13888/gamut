//! The IPTC writer — the convenient entry point for encoding IPTC metadata.

use gamut_core::Result;
use gamut_xmp::XmpProperty;

use crate::iim::IimBlock;
use crate::irb::PhotoshopIrb;
use crate::photo_metadata::PhotoMetadata;

/// Writer for IPTC metadata.
///
/// - [`IptcWriter::write_irb`] serializes IIM datasets into a Photoshop `8BIM` resource stream.
/// - [`IptcWriter::write_xmp_properties`] hands the IPTC properties back as XMP properties to merge
///   into a packet (serialized by [`gamut_xmp`], issue #34).
///
/// To project an IPTC view to the legacy IIM carrier, use
/// [`IimXmpReconciler::to_iim`](crate::reconcile::IimXmpReconciler::to_iim) and then `write_irb`.
#[derive(Debug, Clone, Copy, Default)]
pub struct IptcWriter;

impl IptcWriter {
    /// Creates a new writer.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// Serializes IIM datasets into a Photoshop image-resource (`8BIM`) stream holding a single
    /// `0x0404` resource.
    ///
    /// The output round-trips through [`IptcReader::read_irb`](crate::reader::IptcReader::read_irb).
    ///
    /// # Errors
    ///
    /// Returns an error if a dataset value is too large to serialize (see [`IimBlock::encode`]).
    pub fn write_irb(&self, block: &IimBlock) -> Result<Vec<u8>> {
        PhotoshopIrb::with_iptc(block.encode()?).encode()
    }

    /// Returns the IPTC Photo Metadata as XMP properties, ready to merge into an XMP packet.
    #[must_use]
    pub fn write_xmp_properties(&self, pm: &PhotoMetadata) -> Vec<XmpProperty> {
        pm.to_xmp_properties()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iim::IimDataSet;
    use crate::reader::IptcReader;

    #[test]
    fn write_irb_roundtrips_through_read_irb() {
        let block = IimBlock {
            datasets: vec![
                IimDataSet {
                    record: 2,
                    dataset: 0,
                    data: vec![0x00, 0x04],
                },
                IimDataSet {
                    record: 2,
                    dataset: 25,
                    data: b"sky".to_vec(),
                },
                IimDataSet {
                    record: 2,
                    dataset: 25,
                    data: b"sea".to_vec(),
                },
            ],
        };
        let bytes = IptcWriter::new().write_irb(&block).unwrap();
        let read = IptcReader::new().read_irb(&bytes).unwrap();
        assert_eq!(read, Some(block));
    }

    #[test]
    fn read_irb_without_iptc_resource_is_none() {
        let irb = PhotoshopIrb {
            blocks: vec![crate::irb::IrbBlock {
                resource_id: 0x03ED,
                name: String::new(),
                data: vec![1, 2, 3, 4],
            }],
        };
        let bytes = irb.encode().unwrap();
        assert_eq!(IptcReader::new().read_irb(&bytes).unwrap(), None);
    }

    #[test]
    fn write_xmp_properties_returns_the_properties() {
        let mut pm = PhotoMetadata::new();
        pm.set_headline("Breaking news");
        let props = IptcWriter::new().write_xmp_properties(&pm);
        assert_eq!(props, pm.properties);
    }
}
