//! The IPTC reader — the read-side entry point: decode the legacy carrier and merge carriers.

use gamut_core::Result;
use gamut_xmp::XmpMeta;

use crate::iim::IimBlock;
use crate::irb::PhotoshopIrb;
use crate::photo_metadata::PhotoMetadata;
use crate::reconcile;

/// Which carrier wins when both hold a mapped field with differing values.
///
/// The IPTC guidelines call for keeping the carriers in sync but prescribe no single winner;
/// this policy is gamut's explicit knob. Marked `#[non_exhaustive]`: finer-grained policies may
/// be added post-1.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ConflictPolicy {
    /// The modern XMP value wins (the default; XMP is the authoritative modern carrier, matching
    /// exiv2/exiftool de-facto behaviour).
    #[default]
    XmpWins,
    /// The legacy IIM value wins.
    IimWins,
}

/// A per-field disagreement between the two carriers, reported by [`IptcReader::conflicts`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FieldConflict {
    /// The XMP property name of the field in conflict (e.g. `City`).
    pub field: &'static str,
    /// The value(s) read from the IIM carrier.
    pub iim: Vec<String>,
    /// The value(s) read from the XMP carrier.
    pub xmp: Vec<String>,
}

/// Reader for IPTC metadata.
///
/// - [`IptcReader::read_irb`] / [`IimBlock::parse`] decode the legacy IIM carrier.
/// - [`IptcReader::read`] merges whichever carriers are present into one [`PhotoMetadata`] view,
///   applying the reader's [`ConflictPolicy`]: XMP-only properties are preserved, an IIM-only
///   mapped field is adopted, and a disagreement is resolved by the policy.
/// - [`IptcReader::conflicts`] reports the disagreements without resolving them.
///
/// Reader inputs are the *carriers* ([`IimBlock`], [`XmpMeta`]); [`PhotoMetadata`] is the unified
/// output. Obtaining an [`XmpMeta`] from raw XMP packet bytes is [`gamut_xmp`]'s responsibility
/// (issue #34); interpreting a graph you already hold is [`PhotoMetadata::from_xmp`].
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

    /// Sets the conflict policy applied by [`IptcReader::read`] and returns the reader.
    #[must_use]
    pub fn policy(mut self, policy: ConflictPolicy) -> Self {
        self.policy = policy;
        self
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

    /// Merges whichever carriers are supplied into one unified [`PhotoMetadata`] view, applying the
    /// reader's conflict policy.
    #[must_use]
    pub fn read(&self, iim: Option<&IimBlock>, xmp: Option<&XmpMeta>) -> PhotoMetadata {
        let pm = xmp.map(PhotoMetadata::from_xmp);
        reconcile::merge(self.policy, iim, pm.as_ref())
    }

    /// Reports the mapped fields on which the two carriers disagree (both present, differing
    /// values), without resolving them.
    #[must_use]
    pub fn conflicts(&self, iim: &IimBlock, xmp: &XmpMeta) -> Vec<FieldConflict> {
        reconcile::conflicts(iim, &PhotoMetadata::from_xmp(xmp))
    }
}

#[cfg(test)]
mod tests {
    use gamut_xmp::{XmpProperty, XmpValue};

    use super::*;
    use crate::iim::IimDataSet;

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
            IptcReader::new()
                .policy(ConflictPolicy::IimWins)
                .read(Some(&iim), Some(&meta))
                .city(),
            Some("Kyoto")
        );
    }

    #[test]
    fn conflicts_reports_carrier_disagreements() {
        let iim = IimBlock {
            datasets: vec![IimDataSet {
                record: 2,
                dataset: 90,
                data: b"Kyoto".to_vec(),
            }],
        };
        let conflicts = IptcReader::new().conflicts(&iim, &xmp_with_city("Tokyo"));
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "City");
        assert_eq!(conflicts[0].iim, vec!["Kyoto"]);
        assert_eq!(conflicts[0].xmp, vec!["Tokyo"]);
        // Agreeing carriers report nothing.
        assert!(
            IptcReader::new()
                .conflicts(&iim, &xmp_with_city("Kyoto"))
                .is_empty()
        );
    }
}
