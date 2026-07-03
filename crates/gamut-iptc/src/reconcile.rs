//! Reconciliation between the legacy IIM and the modern XMP representations.
//!
//! An image may carry the same datum in legacy IIM, in IPTC-Core XMP, or in both with conflicting
//! values. This module applies the IPTC mapping guidelines' precedence rules (via [`ConflictPolicy`])
//! to merge the two carriers into one coherent [`PhotoMetadata`] view, and to project that view back
//! to a consistent IIM block. The field mapping is the authoritative table in [`crate::schema`].

use gamut_core::{Error, Result};

use crate::charset::IimCharset;
use crate::date;
use crate::iim::{IimBlock, IimDataSet, IimTagInfo};
use crate::photo_metadata::PhotoMetadata;
use crate::schema::{FieldMap, MAP, XmpShape};

/// Which carrier wins when both hold a mapped field with differing values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConflictPolicy {
    /// The modern XMP value wins (the default; XMP is the authoritative modern carrier, matching
    /// exiv2/exiftool de-facto behaviour).
    #[default]
    XmpWins,
    /// The legacy IIM value wins.
    IimWins,
}

/// A per-field disagreement between the two carriers, reported by [`IimXmpReconciler::conflicts`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldConflict {
    /// The XMP property name of the field in conflict (e.g. `City`).
    pub field: &'static str,
    /// The value(s) read from the IIM carrier.
    pub iim: Vec<String>,
    /// The value(s) read from the XMP carrier.
    pub xmp: Vec<String>,
}

/// Reconciler between legacy IIM datasets and IPTC Photo Metadata (XMP).
///
/// This is the crate's keystone: applying the IPTC mapping guidelines' precedence rules to merge the
/// two carriers into one coherent view, and to write both consistently.
#[derive(Debug, Clone, Copy, Default)]
pub struct IimXmpReconciler {
    /// The precedence policy applied when both carriers disagree on a field.
    pub policy: ConflictPolicy,
}

impl IimXmpReconciler {
    /// A reconciler with the default policy ([`ConflictPolicy::XmpWins`]).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A reconciler with an explicit conflict policy.
    #[must_use]
    pub fn with_policy(policy: ConflictPolicy) -> Self {
        Self { policy }
    }

    /// Merges whichever carriers are present into one unified [`PhotoMetadata`] (XMP) view.
    ///
    /// XMP-only properties are preserved. For each mapped field present in IIM, the value is adopted
    /// when XMP lacks it, kept when both agree, and otherwise resolved by [`IimXmpReconciler::policy`].
    #[must_use]
    pub fn merge(&self, iim: Option<&IimBlock>, xmp: Option<&PhotoMetadata>) -> PhotoMetadata {
        let mut out = xmp.cloned().unwrap_or_default();
        let Some(iim) = iim else {
            return out;
        };
        let charset = IimCharset::detect(iim).unwrap_or(IimCharset::Latin1);
        for row in MAP {
            let iim_vals = read_iim_field(iim, row, charset);
            if iim_vals.is_empty() {
                continue;
            }
            let xmp_vals = out.get_field(&row.xmp);
            if xmp_vals.is_empty() {
                out.set_field(&row.xmp, &iim_vals); // IIM-only: adopt it
            } else if xmp_vals != iim_vals && self.policy == ConflictPolicy::IimWins {
                out.set_field(&row.xmp, &iim_vals); // conflict, IIM wins
            }
            // else: XMP present and (equal, or XmpWins) -> keep what's already in `out`.
        }
        out
    }

    /// Projects a unified [`PhotoMetadata`] to an IIM block, encoding text with `charset`.
    ///
    /// Emits the mandatory `2:00` Record Version and, for UTF-8, the `1:90` coded-character-set
    /// escape, when any mapped field is present.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if a value cannot be encoded in `charset` or exceeds the
    /// dataset's maximum octet length — gamut never silently truncates.
    pub fn to_iim(&self, pm: &PhotoMetadata, charset: IimCharset) -> Result<IimBlock> {
        let mut fields = Vec::new();
        for row in MAP {
            let vals = pm.get_field(&row.xmp);
            if vals.is_empty() {
                continue;
            }
            if row.xmp.shape == XmpShape::DateTime {
                if let Some((date, time)) = date::iso_to_iim(&vals[0]) {
                    push_dataset(&mut fields, 2, 55, date)?;
                    if let Some(time) = time {
                        push_dataset(&mut fields, 2, 60, time)?;
                    }
                }
            } else {
                let (record, dataset) = row.iim[0];
                for v in &vals {
                    push_dataset(&mut fields, record, dataset, charset.encode(v)?)?;
                }
            }
        }

        let mut datasets = Vec::new();
        if !fields.is_empty() {
            if let Some(escape) = charset.escape_sequence() {
                datasets.push(IimDataSet {
                    record: 1,
                    dataset: 90,
                    data: escape.to_vec(),
                });
            }
            datasets.push(IimDataSet {
                record: 2,
                dataset: 0,
                data: vec![0, 4],
            }); // Record Version 4
            datasets.extend(fields);
        }
        Ok(IimBlock { datasets })
    }

    /// Reports the mapped fields on which the two carriers disagree (both present, differing values).
    #[must_use]
    pub fn conflicts(&self, iim: &IimBlock, xmp: &PhotoMetadata) -> Vec<FieldConflict> {
        let charset = IimCharset::detect(iim).unwrap_or(IimCharset::Latin1);
        let mut out = Vec::new();
        for row in MAP {
            let iim_vals = read_iim_field(iim, row, charset);
            let xmp_vals = xmp.get_field(&row.xmp);
            if !iim_vals.is_empty() && !xmp_vals.is_empty() && iim_vals != xmp_vals {
                out.push(FieldConflict {
                    field: row.xmp.name,
                    iim: iim_vals,
                    xmp: xmp_vals,
                });
            }
        }
        out
    }
}

/// Reads a mapped field's value(s) from the IIM block, decoded with `charset`.
fn read_iim_field(iim: &IimBlock, row: &FieldMap, charset: IimCharset) -> Vec<String> {
    if row.xmp.shape == XmpShape::DateTime {
        let find = |dataset| {
            iim.datasets
                .iter()
                .find(|d| d.record == 2 && d.dataset == dataset)
        };
        return match find(55) {
            Some(d) => date::iim_to_iso(&d.data, find(60).map(|t| t.data.as_slice()))
                .map(|s| vec![s])
                .unwrap_or_default(),
            None => Vec::new(),
        };
    }
    let (record, dataset) = row.iim[0];
    let decoded: Vec<String> = iim
        .datasets
        .iter()
        .filter(|d| d.record == record && d.dataset == dataset)
        .filter_map(|d| charset.decode(&d.data).ok())
        .collect();
    match row.xmp.shape {
        XmpShape::Bag | XmpShape::Seq => decoded,
        _ => decoded.into_iter().take(1).collect(),
    }
}

/// Appends a dataset, rejecting a value that exceeds the dataset's maximum octet length.
fn push_dataset(out: &mut Vec<IimDataSet>, record: u8, dataset: u8, data: Vec<u8>) -> Result<()> {
    if let Some(info) = IimTagInfo::lookup(record, dataset)
        && data.len() > usize::from(info.max_octets)
    {
        return Err(Error::InvalidInput(
            "IPTC IIM: value exceeds the dataset's maximum length",
        ));
    }
    out.push(IimDataSet {
        record,
        dataset,
        data,
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ds(record: u8, dataset: u8, data: &[u8]) -> IimDataSet {
        IimDataSet {
            record,
            dataset,
            data: data.to_vec(),
        }
    }

    #[test]
    fn merge_only_iim_promotes_to_xmp() {
        let iim = IimBlock {
            datasets: vec![ds(2, 90, b"Paris"), ds(2, 25, b"sky"), ds(2, 25, b"sea")],
        };
        let pm = IimXmpReconciler::new().merge(Some(&iim), None);
        assert_eq!(pm.city(), Some("Paris"));
        assert_eq!(pm.keywords(), vec!["sky", "sea"]);
    }

    #[test]
    fn merge_only_xmp_is_unchanged() {
        let mut xmp = PhotoMetadata::new();
        xmp.set_city("Berlin");
        xmp.set_usage_terms("CC-BY"); // XMP-only field is preserved
        let pm = IimXmpReconciler::new().merge(None, Some(&xmp));
        assert_eq!(pm, xmp);
    }

    #[test]
    fn merge_conflict_respects_policy() {
        let iim = IimBlock {
            datasets: vec![ds(2, 90, b"Lyon")],
        };
        let mut xmp = PhotoMetadata::new();
        xmp.set_city("Paris");

        let xmp_wins = IimXmpReconciler::new().merge(Some(&iim), Some(&xmp));
        assert_eq!(xmp_wins.city(), Some("Paris"));

        let iim_wins =
            IimXmpReconciler::with_policy(ConflictPolicy::IimWins).merge(Some(&iim), Some(&xmp));
        assert_eq!(iim_wins.city(), Some("Lyon"));
    }

    #[test]
    fn merge_agreeing_carriers_keep_value() {
        let iim = IimBlock {
            datasets: vec![ds(2, 90, b"Oslo")],
        };
        let mut xmp = PhotoMetadata::new();
        xmp.set_city("Oslo");
        let pm =
            IimXmpReconciler::with_policy(ConflictPolicy::IimWins).merge(Some(&iim), Some(&xmp));
        assert_eq!(pm.city(), Some("Oslo"));
        assert_eq!(pm.properties.len(), 1);
    }

    #[test]
    fn to_iim_emits_version_and_fields() {
        let mut pm = PhotoMetadata::new();
        pm.set_city("Paris");
        pm.set_keywords(&["sky", "sea"]);
        let block = IimXmpReconciler::new()
            .to_iim(&pm, IimCharset::Latin1)
            .unwrap();
        assert!(block.datasets.contains(&ds(2, 0, &[0, 4]))); // Record Version
        assert!(block.datasets.contains(&ds(2, 90, b"Paris")));
        assert_eq!(
            block.datasets.iter().filter(|d| d.dataset == 25).count(),
            2 // one dataset per keyword
        );
    }

    #[test]
    fn to_iim_writes_utf8_escape_only_when_needed() {
        let mut pm = PhotoMetadata::new();
        pm.set_city("Köln");
        let utf8 = IimXmpReconciler::new()
            .to_iim(&pm, IimCharset::Utf8)
            .unwrap();
        assert_eq!(utf8.datasets[0], ds(1, 90, &IimCharset::UTF8_ESCAPE));
        // An empty view emits nothing at all — not even the version/escape.
        let empty = IimXmpReconciler::new()
            .to_iim(&PhotoMetadata::new(), IimCharset::Utf8)
            .unwrap();
        assert!(empty.datasets.is_empty());
    }

    #[test]
    fn to_iim_rejects_overlong_and_unencodable_values() {
        let mut too_long = PhotoMetadata::new();
        too_long.set_city(&"x".repeat(33)); // City max is 32 octets
        assert!(
            IimXmpReconciler::new()
                .to_iim(&too_long, IimCharset::Latin1)
                .is_err()
        );

        let mut non_latin1 = PhotoMetadata::new();
        non_latin1.set_city("Köln"); // not representable in Latin-1? 'ö' is U+00F6, IS Latin-1
        non_latin1.set_headline("€"); // U+20AC is not Latin-1
        assert!(
            IimXmpReconciler::new()
                .to_iim(&non_latin1, IimCharset::Latin1)
                .is_err()
        );
    }

    #[test]
    fn datetime_splits_and_joins() {
        let mut pm = PhotoMetadata::new();
        pm.set_simple(
            crate::schema::ns::PHOTOSHOP,
            "DateCreated",
            "1990-01-27T13:30:15+01:00",
        );
        let block = IimXmpReconciler::new()
            .to_iim(&pm, IimCharset::Latin1)
            .unwrap();
        assert!(block.datasets.contains(&ds(2, 55, b"19900127")));
        assert!(block.datasets.contains(&ds(2, 60, b"133015+0100")));
        // ...and back the other way.
        let merged = IimXmpReconciler::new().merge(Some(&block), None);
        assert_eq!(
            merged.simple(crate::schema::ns::PHOTOSHOP, "DateCreated"),
            Some("1990-01-27T13:30:15+01:00")
        );
    }

    #[test]
    fn conflicts_lists_only_disagreements() {
        let iim = IimBlock {
            datasets: vec![ds(2, 90, b"Lyon"), ds(2, 101, b"France")],
        };
        let mut xmp = PhotoMetadata::new();
        xmp.set_city("Paris"); // conflicts
        xmp.set_country("France"); // agrees
        let conflicts = IimXmpReconciler::new().conflicts(&iim, &xmp);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].field, "City");
        assert_eq!(conflicts[0].iim, vec!["Lyon"]);
        assert_eq!(conflicts[0].xmp, vec!["Paris"]);
    }
}
