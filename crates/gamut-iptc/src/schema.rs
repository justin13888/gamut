//! The IPTC Photo Metadata schema: XMP namespaces and the IIM↔XMP property map.
//!
//! IPTC Core/Extension fields are defined *as* XMP properties (IPTC Photo Metadata Standard
//! 2025.1). This module pins the namespace URIs and the authoritative mapping between legacy IIM
//! datasets and their XMP properties, derived from the IPTC machine-readable technical reference
//! (`references/iptc/iptc-pmd-techreference_2025.1.json`, the `ipmd_top` entries that carry an
//! `IIMid`). The map drives the IIM↔XMP reconciliation; the per-dataset octet limits are not
//! duplicated here — they come from [`crate::iim::IimTagInfo::lookup`].

/// XMP namespace URIs used by IPTC Photo Metadata (verified against the IPTC Photo Metadata
/// Standard 2025.1).
pub mod ns {
    /// Dublin Core — `dc:` (title, creator, description, subject, rights).
    pub const DC: &str = "http://purl.org/dc/elements/1.1/";
    /// Adobe Photoshop — `photoshop:` (City, Country, Headline, Credit, …).
    pub const PHOTOSHOP: &str = "http://ns.adobe.com/photoshop/1.0/";
    /// XMP Rights Management — `xmpRights:` (UsageTerms, WebStatement).
    pub const XMP_RIGHTS: &str = "http://ns.adobe.com/xap/1.0/rights/";
    /// IPTC Photo Metadata Core — `Iptc4xmpCore:`.
    pub const IPTC_CORE: &str = "http://iptc.org/std/Iptc4xmpCore/1.0/xmlns/";
    /// IPTC Photo Metadata Extension — `Iptc4xmpExt:`.
    pub const IPTC_EXT: &str = "http://iptc.org/std/Iptc4xmpExt/2008-02-29/";
}

/// The namespaces gamut treats as IPTC-relevant when extracting [`crate::PhotoMetadata`] from a full
/// XMP graph (see [`crate::PhotoMetadata::from_xmp`]).
pub const IPTC_NAMESPACES: &[&str] = &[
    ns::DC,
    ns::PHOTOSHOP,
    ns::XMP_RIGHTS,
    ns::IPTC_CORE,
    ns::IPTC_EXT,
];

/// How an IPTC field is shaped as an XMP value.
///
/// Marked `#[non_exhaustive]`: IPTC Extension structures may need further shapes post-1.0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum XmpShape {
    /// A simple text property (e.g. `photoshop:City`).
    SimpleText,
    /// A language-alternative array; gamut reads/writes the `x-default` alternative (e.g.
    /// `dc:description`).
    LangAlt,
    /// An unordered `rdf:Bag` of text (e.g. `dc:subject` keywords).
    Bag,
    /// An ordered `rdf:Seq` of text (e.g. `dc:creator`).
    Seq,
    /// A date-time text property assembled from two IIM datasets (e.g. `photoshop:DateCreated`).
    DateTime,
}

/// One XMP property's identity and shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct XmpField {
    /// The namespace URI (one of [`ns`]).
    pub ns: &'static str,
    /// The local property name.
    pub name: &'static str,
    /// How the value is shaped as XMP.
    pub shape: XmpShape,
}

/// The mapping of one IPTC field between its IIM dataset(s) and its XMP property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldMap {
    /// The IIM `(record, dataset)` tag(s). Usually one; `DateTime` maps the date (2:55) and time
    /// (2:60) datasets together.
    pub iim: &'static [(u8, u8)],
    /// The XMP property this field maps to.
    pub xmp: XmpField,
}

use XmpShape::{Bag, DateTime, LangAlt, Seq, SimpleText};

const fn field(ns: &'static str, name: &'static str, shape: XmpShape) -> XmpField {
    XmpField { ns, name, shape }
}

/// The authoritative IIM↔XMP map (the 20 `ipmd_top` properties that carry an `IIMid`).
///
/// Each entry pairs the IIM dataset(s) with the XMP property and its shape. `2:04`/`2:85` are
/// repeatable on the IIM wire but map to single XMP properties (gamut reconciles the first value);
/// `2:55`+`2:60` together form the `photoshop:DateCreated` date-time.
///
/// Together with [`crate::PhotoMetadata::get_field`]/[`set_field`](crate::PhotoMetadata::set_field)
/// this enables generic, table-driven access to every mapped field:
///
/// ```
/// use gamut_iptc::{PhotoMetadata, schema::FIELD_MAP};
///
/// let mut pm = PhotoMetadata::new();
/// pm.set_city("Lyon");
/// let present: Vec<&str> = FIELD_MAP
///     .iter()
///     .filter(|row| !pm.get_field(&row.xmp).is_empty())
///     .map(|row| row.xmp.name)
///     .collect();
/// assert_eq!(present, ["City"]);
/// ```
pub const FIELD_MAP: &[FieldMap] = &[
    FieldMap {
        iim: &[(2, 4)],
        xmp: field(ns::IPTC_CORE, "IntellectualGenre", SimpleText),
    },
    FieldMap {
        iim: &[(2, 5)],
        xmp: field(ns::DC, "title", LangAlt),
    },
    FieldMap {
        iim: &[(2, 12)],
        xmp: field(ns::IPTC_CORE, "SubjectCode", Bag),
    },
    FieldMap {
        iim: &[(2, 25)],
        xmp: field(ns::DC, "subject", Bag),
    },
    FieldMap {
        iim: &[(2, 40)],
        xmp: field(ns::PHOTOSHOP, "Instructions", SimpleText),
    },
    FieldMap {
        iim: &[(2, 55), (2, 60)],
        xmp: field(ns::PHOTOSHOP, "DateCreated", DateTime),
    },
    FieldMap {
        iim: &[(2, 80)],
        xmp: field(ns::DC, "creator", Seq),
    },
    FieldMap {
        iim: &[(2, 85)],
        xmp: field(ns::PHOTOSHOP, "AuthorsPosition", SimpleText),
    },
    FieldMap {
        iim: &[(2, 90)],
        xmp: field(ns::PHOTOSHOP, "City", SimpleText),
    },
    FieldMap {
        iim: &[(2, 92)],
        xmp: field(ns::IPTC_CORE, "Location", SimpleText),
    },
    FieldMap {
        iim: &[(2, 95)],
        xmp: field(ns::PHOTOSHOP, "State", SimpleText),
    },
    FieldMap {
        iim: &[(2, 100)],
        xmp: field(ns::IPTC_CORE, "CountryCode", SimpleText),
    },
    FieldMap {
        iim: &[(2, 101)],
        xmp: field(ns::PHOTOSHOP, "Country", SimpleText),
    },
    FieldMap {
        iim: &[(2, 103)],
        xmp: field(ns::PHOTOSHOP, "TransmissionReference", SimpleText),
    },
    FieldMap {
        iim: &[(2, 105)],
        xmp: field(ns::PHOTOSHOP, "Headline", SimpleText),
    },
    FieldMap {
        iim: &[(2, 110)],
        xmp: field(ns::PHOTOSHOP, "Credit", SimpleText),
    },
    FieldMap {
        iim: &[(2, 115)],
        xmp: field(ns::PHOTOSHOP, "Source", SimpleText),
    },
    FieldMap {
        iim: &[(2, 116)],
        xmp: field(ns::DC, "rights", LangAlt),
    },
    FieldMap {
        iim: &[(2, 120)],
        xmp: field(ns::DC, "description", LangAlt),
    },
    FieldMap {
        iim: &[(2, 122)],
        xmp: field(ns::PHOTOSHOP, "CaptionWriter", SimpleText),
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mapped_iim_dataset_has_tag_info() {
        // The map and the IIM known-tag table must agree on which datasets are modelled.
        for row in FIELD_MAP {
            for &(record, dataset) in row.iim {
                assert!(
                    crate::iim::IimTagInfo::lookup(record, dataset).is_some(),
                    "{record}:{dataset} is mapped but missing from the IIM tag table"
                );
            }
        }
    }

    #[test]
    fn datetime_is_the_only_two_dataset_row() {
        for row in FIELD_MAP {
            let expected = if row.xmp.shape == XmpShape::DateTime {
                2
            } else {
                1
            };
            assert_eq!(row.iim.len(), expected, "{} arity", row.xmp.name);
        }
    }

    #[test]
    fn known_namespaces_are_iptc_relevant() {
        for row in FIELD_MAP {
            assert!(IPTC_NAMESPACES.contains(&row.xmp.ns));
        }
    }
}
