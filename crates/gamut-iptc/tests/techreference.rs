//! Drift guard: pins gamut's hand-transcribed IIM↔XMP tables to the IPTC machine-readable
//! technical reference vendored in `references/iptc/iptc-pmd-techreference_2025.1.json`.
//!
//! [`gamut_iptc::schema::FIELD_MAP`] and the [`gamut_iptc::IimTagInfo`] table are transcribed from
//! that file (the `ipmd_top` entries carrying an `IIMid`). These tests re-derive the mapping from
//! the JSON at test time and compare, so any transcription slip — or a future IPTC release changing
//! the reference — fails loudly instead of silently drifting. The versioned filename makes bumping
//! to a new IPTC edition a deliberate act that re-runs this gate.

use std::collections::BTreeMap;

use gamut_iptc::schema::{FIELD_MAP, XmpShape, ns};
use gamut_iptc::{IimTagInfo, PhotoMetadata};
use serde_json::Value;

/// Where the vendored references disagree, the crate follows `iim-4.2.pdf`. Each row pins BOTH
/// values — `((record, dataset), crate max_octets, JSON IIMmaxbytes, why)` — so the exception
/// self-invalidates if either source or the crate changes.
const LIMIT_EXCEPTIONS: &[((u8, u8), u16, u64, &str)] = &[(
    (2, 4),
    68,
    64,
    "IIM 4.2 wire form: 3-digit reference number + ':' + up to 64 octets of text = 68 octets; \
     the PMD JSON's IIMmaxbytes counts only the text part",
)];

fn techreference() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../references/iptc/iptc-pmd-techreference_2025.1.json");
    let json = std::fs::read_to_string(&path)
        .expect("vendored IPTC tech reference (references/iptc) must be readable");
    serde_json::from_str(&json).expect("tech reference must be valid JSON")
}

/// The `ipmd_top` entries that carry an `IIMid`, as `(record, dataset) -> entry`.
fn iim_mapped_entries(doc: &Value) -> BTreeMap<(u8, u8), &Value> {
    doc["ipmd_top"]
        .as_object()
        .expect("ipmd_top is an object")
        .values()
        .filter_map(|entry| {
            let iim_id = entry.get("IIMid")?.as_str()?;
            let (record, dataset) = iim_id.split_once(':').expect("IIMid is R:DD");
            let key = (
                record.parse().expect("IIMid record is a u8"),
                dataset.parse().expect("IIMid dataset is a u8"),
            );
            Some((key, entry))
        })
        .collect()
}

/// gamut's namespace URI -> the prefix the tech reference uses in `XMPid`.
fn ns_prefix(uri: &str) -> &'static str {
    match uri {
        _ if uri == ns::DC => "dc",
        _ if uri == ns::PHOTOSHOP => "photoshop",
        _ if uri == ns::XMP_RIGHTS => "xmpRights",
        _ if uri == ns::IPTC_CORE => "Iptc4xmpCore",
        _ if uri == ns::IPTC_EXT => "Iptc4xmpExt",
        _ => panic!("FIELD_MAP references a namespace outside the IPTC set: {uri}"),
    }
}

/// The IIM↔XMP mapping must be a bijection between FIELD_MAP and the JSON's IIMid-bearing rows:
/// no missing rows, no extra rows, no XMPid mismatch.
#[test]
fn field_map_matches_techreference_iim_mapping() {
    let doc = techreference();
    let json_rows: BTreeMap<(u8, u8), String> = iim_mapped_entries(&doc)
        .into_iter()
        .map(|(key, entry)| {
            let xmp_id = entry["XMPid"].as_str().expect("XMPid is a string");
            (key, xmp_id.to_owned())
        })
        .collect();

    let map_rows: BTreeMap<(u8, u8), String> = FIELD_MAP
        .iter()
        .map(|row| {
            // The JSON records DateCreated under 2:55 only; 2:60 (Time Created) is the crate's
            // companion time half, asserted separately below.
            let (record, dataset) = row.iim[0];
            let xmp_id = format!("{}:{}", ns_prefix(row.xmp.ns), row.xmp.name);
            ((record, dataset), xmp_id)
        })
        .collect();

    assert_eq!(
        map_rows, json_rows,
        "FIELD_MAP and the tech reference disagree on the IIM<->XMP mapping"
    );

    // The only multi-dataset row is the 2:55+2:60 DateCreated pair.
    for row in FIELD_MAP {
        match row.xmp.shape {
            XmpShape::DateTime => assert_eq!(row.iim, [(2, 55), (2, 60)], "DateCreated datasets"),
            _ => assert_eq!(
                row.iim.len(),
                1,
                "{} maps exactly one dataset",
                row.xmp.name
            ),
        }
    }
}

/// Every mapped dataset's octet limit must match the JSON's `IIMmaxbytes`, modulo the documented
/// exception list (where the crate follows the IIM 4.2 PDF instead).
#[test]
fn tag_table_octet_limits_match_techreference() {
    let doc = techreference();
    for (key, entry) in iim_mapped_entries(&doc) {
        let (record, dataset) = key;
        let info = IimTagInfo::lookup(record, dataset)
            .unwrap_or_else(|| panic!("{record}:{dataset} is IIM-mapped but not in the tag table"));
        // 2:55 Date Created carries no IIMmaxbytes in the JSON (its length is fixed by form).
        let Some(json_max) = entry.get("IIMmaxbytes").and_then(Value::as_u64) else {
            continue;
        };
        if let Some(&(_, crate_max, exc_json_max, why)) =
            LIMIT_EXCEPTIONS.iter().find(|(k, ..)| *k == key)
        {
            // Pin both sides so the exception self-invalidates when either source moves.
            assert_eq!(info.max_octets, crate_max, "{record}:{dataset}: {why}");
            assert_eq!(json_max, exc_json_max, "{record}:{dataset}: {why}");
        } else {
            assert_eq!(
                u64::from(info.max_octets),
                json_max,
                "{record}:{dataset} octet limit drifted from the tech reference"
            );
        }
    }
}

/// The XMP shape of each mapped field must agree with the JSON's occurrence and type columns.
#[test]
fn shapes_match_techreference_occurrence_and_type() {
    let doc = techreference();
    let entries = iim_mapped_entries(&doc);
    for row in FIELD_MAP {
        let entry = entries[&row.iim[0]];
        let multi = entry["propoccurrence"].as_str() == Some("multi");
        let shape_multi = matches!(row.xmp.shape, XmpShape::Bag | XmpShape::Seq);
        assert_eq!(
            multi, shape_multi,
            "{}: propoccurrence vs shape mismatch",
            row.xmp.name
        );
        // AltLang struct rows must be LangAlt; the date-time row must be DateTime.
        if entry.get("dataformat").and_then(Value::as_str) == Some("AltLang") {
            assert_eq!(row.xmp.shape, XmpShape::LangAlt, "{}", row.xmp.name);
        }
        if entry.get("dataformat").and_then(Value::as_str) == Some("date-time") {
            assert_eq!(row.xmp.shape, XmpShape::DateTime, "{}", row.xmp.name);
        }
    }

    // Sanity: the generic field accessors respect the reference-mandated container kinds — a
    // multi property written through set_field must come back multi-valued.
    let mut pm = PhotoMetadata::new();
    for row in FIELD_MAP {
        if matches!(row.xmp.shape, XmpShape::Bag | XmpShape::Seq) {
            pm.set_field(&row.xmp, &["a", "b"]);
            assert_eq!(pm.get_field(&row.xmp), ["a", "b"], "{}", row.xmp.name);
        }
    }
}
