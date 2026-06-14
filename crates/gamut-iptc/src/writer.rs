//! The IPTC writer — the convenient entry point for encoding IPTC metadata.

use gamut_core::Result;

use crate::iim::IimBlock;
use crate::irb::PhotoshopIrb;

/// Writer for IPTC metadata.
///
/// For the legacy carrier, [`IptcWriter::write_irb`] serializes IIM datasets into a Photoshop
/// image-resource stream. To produce a bare IIM dataset stream use [`IimBlock::encode`] directly.
///
/// The modern IPTC Photo Metadata (Core/Extension) path, projecting back to XMP properties, is
/// added in [`crate::photo_metadata`] (see issue #34).
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
        // A resource stream with only a non-IPTC block.
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
}
