//! The IPTC reader — the convenient entry point for decoding IPTC metadata.

use gamut_core::Result;
use gamut_xmp::XmpMeta;

use crate::iim::IimBlock;
use crate::irb::PhotoshopIrb;
use crate::photo_metadata::PhotoMetadata;
use crate::reconcile::{ConflictPolicy, IimXmpReconciler};

/// Reader for IPTC metadata.
///
/// - [`IptcReader::read_irb`] / [`IimBlock::parse`] decode the legacy IIM carrier.
/// - [`IptcReader::read_xmp`] interprets a parsed XMP graph as IPTC Photo Metadata.
/// - [`IptcReader::read`] merges whichever carriers are present into one view, applying the reader's
///   [`ConflictPolicy`].
///
/// Obtaining an [`XmpMeta`] from raw XMP packet bytes is [`gamut_xmp`]'s responsibility (issue #34).
#[derive(Debug, Clone, Copy, Default)]
pub struct IptcReader {
    policy: ConflictPolicy,
}

impl IptcReader {
    /// Creates a reader with the default conflict policy ([`ConflictPolicy::XmpWins`]).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a reader with an explicit conflict policy for [`IptcReader::read`].
    #[must_use]
    pub fn with_policy(policy: ConflictPolicy) -> Self {
        Self { policy }
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

    /// Interprets an already-parsed XMP graph as IPTC Photo Metadata (the `dc:`/`photoshop:`/
    /// `Iptc4xmp*:` properties).
    #[must_use]
    pub fn read_xmp(&self, meta: &XmpMeta) -> PhotoMetadata {
        PhotoMetadata::from_xmp(meta)
    }

    /// Merges whichever carriers are supplied into one unified [`PhotoMetadata`] view, applying the
    /// reader's conflict policy.
    #[must_use]
    pub fn read(&self, iim: Option<&IimBlock>, xmp: Option<&XmpMeta>) -> PhotoMetadata {
        let pm = xmp.map(PhotoMetadata::from_xmp);
        IimXmpReconciler::with_policy(self.policy).merge(iim, pm.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::iim::IimDataSet;
    use gamut_xmp::{XmpProperty, XmpValue};

    #[test]
    fn read_irb_propagates_resource_stream_errors() {
        assert!(IptcReader::new().read_irb(b"not an irb").is_err());
    }

    #[test]
    fn read_irb_propagates_iim_errors() {
        let irb = PhotoshopIrb::with_iptc(vec![0x00, 0x01]).encode().unwrap();
        assert!(IptcReader::new().read_irb(&irb).is_err());
    }

    fn xmp_with_city(city: &str) -> XmpMeta {
        XmpMeta {
            properties: vec![XmpProperty {
                namespace: crate::schema::ns::PHOTOSHOP.to_owned(),
                name: "City".to_owned(),
                value: XmpValue::Simple(city.to_owned()),
                qualifiers: Vec::new(),
            }],
        }
    }

    #[test]
    fn read_xmp_extracts_photo_metadata() {
        let meta = xmp_with_city("Tokyo");
        assert_eq!(IptcReader::new().read_xmp(&meta).city(), Some("Tokyo"));
    }

    #[test]
    fn read_merges_with_policy() {
        let iim = IimBlock {
            datasets: vec![IimDataSet {
                record: 2,
                dataset: 90,
                data: b"Kyoto".to_vec(),
            }],
        };
        let meta = xmp_with_city("Tokyo");
        // Default policy keeps XMP; IimWins prefers the legacy value.
        assert_eq!(
            IptcReader::new().read(Some(&iim), Some(&meta)).city(),
            Some("Tokyo")
        );
        assert_eq!(
            IptcReader::with_policy(ConflictPolicy::IimWins)
                .read(Some(&iim), Some(&meta))
                .city(),
            Some("Kyoto")
        );
    }
}
