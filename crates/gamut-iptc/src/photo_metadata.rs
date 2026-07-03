//! The modern IPTC Photo Metadata (Core + Extension), expressed over XMP.

use gamut_xmp::{XmpArray, XmpItem, XmpMeta, XmpProperty, XmpValue};

use crate::schema::{IPTC_NAMESPACES, XmpField, XmpShape, ns};

/// IPTC Photo Metadata (Core + Extension), the modern standard.
///
/// These properties are defined *as* XMP (in the `dc:`/`photoshop:`/`Iptc4xmpCore:` namespaces), so
/// the model is carried as [`gamut_xmp`] properties rather than a parallel type hierarchy. Typed
/// accessors cover the well-known Core fields; arbitrary IPTC properties remain accessible through
/// [`PhotoMetadata::properties`].
///
/// gamut-iptc operates on this in-memory graph; parsing/serializing the XMP *packet bytes* is
/// [`gamut_xmp`]'s responsibility (see issue #34). Language-alternative fields (`dc:title`,
/// `dc:rights`, `dc:description`) are read and written as their `x-default` alternative.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PhotoMetadata {
    /// The IPTC Core/Extension properties, as XMP properties in the IPTC namespaces.
    pub properties: Vec<XmpProperty>,
}

fn simple_str(value: &XmpValue) -> Option<&str> {
    match value {
        XmpValue::Simple(s) => Some(s),
        _ => None,
    }
}

impl PhotoMetadata {
    /// Creates an empty set of IPTC Photo Metadata.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Extracts the IPTC-relevant properties (the `dc:`/`photoshop:`/`xmpRights:`/`Iptc4xmp*:`
    /// namespaces) from a parsed XMP graph.
    #[must_use]
    pub fn from_xmp(meta: &XmpMeta) -> Self {
        Self {
            properties: meta
                .properties
                .iter()
                .filter(|p| IPTC_NAMESPACES.contains(&p.namespace.as_str()))
                .cloned()
                .collect(),
        }
    }

    /// The IPTC properties as an XMP graph, ready for serialization by [`gamut_xmp`] (or merging
    /// into a larger graph). The symmetric inverse of [`PhotoMetadata::from_xmp`].
    #[must_use]
    pub fn to_xmp(&self) -> XmpMeta {
        XmpMeta {
            properties: self.properties.clone(),
        }
    }

    fn find(&self, ns: &str, name: &str) -> Option<&XmpProperty> {
        self.properties
            .iter()
            .find(|p| p.namespace == ns && p.name == name)
    }

    fn upsert(&mut self, ns: &str, name: &str, value: XmpValue) {
        if let Some(p) = self
            .properties
            .iter_mut()
            .find(|p| p.namespace == ns && p.name == name)
        {
            p.value = value;
        } else {
            self.properties.push(XmpProperty {
                namespace: ns.to_owned(),
                name: name.to_owned(),
                value,
                qualifiers: Vec::new(),
            });
        }
    }

    pub(crate) fn simple(&self, ns: &str, name: &str) -> Option<&str> {
        simple_str(&self.find(ns, name)?.value)
    }

    pub(crate) fn set_simple(&mut self, ns: &str, name: &str, value: &str) {
        self.upsert(ns, name, XmpValue::Simple(value.to_owned()));
    }

    /// Reads the `x-default` (first) alternative of a language-alternative property.
    pub(crate) fn lang_alt(&self, ns: &str, name: &str) -> Option<&str> {
        match &self.find(ns, name)?.value {
            XmpValue::Array(XmpArray::Alt(items)) => {
                items.iter().find_map(|item| simple_str(&item.value))
            }
            XmpValue::Simple(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn set_lang_alt(&mut self, ns: &str, name: &str, value: &str) {
        let alt = XmpArray::Alt(vec![XmpItem::new(XmpValue::Simple(value.to_owned()))]);
        self.upsert(ns, name, XmpValue::Array(alt));
    }

    pub(crate) fn list(&self, ns: &str, name: &str) -> Vec<&str> {
        match self.find(ns, name).map(|p| &p.value) {
            Some(XmpValue::Array(XmpArray::Bag(items) | XmpArray::Seq(items))) => items
                .iter()
                .filter_map(|item| simple_str(&item.value))
                .collect(),
            _ => Vec::new(),
        }
    }

    pub(crate) fn set_list(&mut self, ns: &str, name: &str, ordered: bool, values: &[&str]) {
        let items = values
            .iter()
            .map(|s| XmpItem::new(XmpValue::Simple((*s).to_owned())))
            .collect();
        let array = if ordered {
            XmpArray::Seq(items)
        } else {
            XmpArray::Bag(items)
        };
        self.upsert(ns, name, XmpValue::Array(array));
    }

    /// Reads a field's value(s) by its schema identity, as owned strings (empty if absent).
    ///
    /// Scalar shapes ([`XmpShape::SimpleText`], [`XmpShape::LangAlt`], [`XmpShape::DateTime`])
    /// yield zero or one element; array shapes yield every item. Together with
    /// [`crate::schema::FIELD_MAP`] this gives generic, table-driven access to every mapped field
    /// without a typed accessor per field.
    #[must_use]
    pub fn get_field(&self, field: &XmpField) -> Vec<String> {
        match field.shape {
            XmpShape::SimpleText | XmpShape::DateTime => self
                .simple(field.ns, field.name)
                .map(|s| vec![s.to_owned()])
                .unwrap_or_default(),
            XmpShape::LangAlt => self
                .lang_alt(field.ns, field.name)
                .map(|s| vec![s.to_owned()])
                .unwrap_or_default(),
            XmpShape::Bag | XmpShape::Seq => self
                .list(field.ns, field.name)
                .iter()
                .map(|s| (*s).to_owned())
                .collect(),
        }
    }

    /// Writes a field's value(s) by its schema identity, storing the RDF container kind the shape
    /// prescribes.
    ///
    /// Scalar shapes take the first value; an empty value list leaves the property unset. The
    /// counterpart of [`PhotoMetadata::get_field`].
    pub fn set_field(&mut self, field: &XmpField, values: &[&str]) {
        match field.shape {
            XmpShape::SimpleText | XmpShape::DateTime => {
                if let Some(v) = values.first() {
                    self.set_simple(field.ns, field.name, v);
                }
            }
            XmpShape::LangAlt => {
                if let Some(v) = values.first() {
                    self.set_lang_alt(field.ns, field.name, v);
                }
            }
            XmpShape::Bag | XmpShape::Seq => {
                self.set_list(field.ns, field.name, field.shape == XmpShape::Seq, values);
            }
        }
    }

    // --- Typed accessors for the well-known IPTC Core fields ---

    /// The headline (`photoshop:Headline`).
    #[must_use]
    pub fn headline(&self) -> Option<&str> {
        self.simple(ns::PHOTOSHOP, "Headline")
    }
    /// Sets the headline (`photoshop:Headline`).
    pub fn set_headline(&mut self, value: &str) {
        self.set_simple(ns::PHOTOSHOP, "Headline", value);
    }

    /// The city (`photoshop:City`).
    #[must_use]
    pub fn city(&self) -> Option<&str> {
        self.simple(ns::PHOTOSHOP, "City")
    }
    /// Sets the city (`photoshop:City`).
    pub fn set_city(&mut self, value: &str) {
        self.set_simple(ns::PHOTOSHOP, "City", value);
    }

    /// The country name (`photoshop:Country`).
    #[must_use]
    pub fn country(&self) -> Option<&str> {
        self.simple(ns::PHOTOSHOP, "Country")
    }
    /// Sets the country name (`photoshop:Country`).
    pub fn set_country(&mut self, value: &str) {
        self.set_simple(ns::PHOTOSHOP, "Country", value);
    }

    /// The ISO country code (`Iptc4xmpCore:CountryCode`).
    #[must_use]
    pub fn country_code(&self) -> Option<&str> {
        self.simple(ns::IPTC_CORE, "CountryCode")
    }
    /// Sets the ISO country code (`Iptc4xmpCore:CountryCode`).
    pub fn set_country_code(&mut self, value: &str) {
        self.set_simple(ns::IPTC_CORE, "CountryCode", value);
    }

    /// The caption/description, `x-default` alternative (`dc:description`).
    #[must_use]
    pub fn caption(&self) -> Option<&str> {
        self.lang_alt(ns::DC, "description")
    }
    /// Sets the caption/description `x-default` alternative (`dc:description`).
    pub fn set_caption(&mut self, value: &str) {
        self.set_lang_alt(ns::DC, "description", value);
    }

    /// The copyright notice, `x-default` alternative (`dc:rights`).
    #[must_use]
    pub fn copyright_notice(&self) -> Option<&str> {
        self.lang_alt(ns::DC, "rights")
    }
    /// Sets the copyright notice `x-default` alternative (`dc:rights`).
    pub fn set_copyright_notice(&mut self, value: &str) {
        self.set_lang_alt(ns::DC, "rights", value);
    }

    /// The title/object name, `x-default` alternative (`dc:title`).
    #[must_use]
    pub fn title(&self) -> Option<&str> {
        self.lang_alt(ns::DC, "title")
    }
    /// Sets the title/object name `x-default` alternative (`dc:title`).
    pub fn set_title(&mut self, value: &str) {
        self.set_lang_alt(ns::DC, "title", value);
    }

    /// The keywords (`dc:subject`).
    #[must_use]
    pub fn keywords(&self) -> Vec<&str> {
        self.list(ns::DC, "subject")
    }
    /// Sets the keywords (`dc:subject`, an unordered bag).
    pub fn set_keywords(&mut self, keywords: &[&str]) {
        self.set_list(ns::DC, "subject", false, keywords);
    }

    /// The creators/by-lines (`dc:creator`).
    #[must_use]
    pub fn creators(&self) -> Vec<&str> {
        self.list(ns::DC, "creator")
    }
    /// Sets the creators/by-lines (`dc:creator`, an ordered sequence).
    pub fn set_creators(&mut self, creators: &[&str]) {
        self.set_list(ns::DC, "creator", true, creators);
    }

    /// The usage terms, `x-default` alternative (`xmpRights:UsageTerms`). This is an XMP-only field
    /// with no IIM equivalent.
    #[must_use]
    pub fn usage_terms(&self) -> Option<&str> {
        self.lang_alt(ns::XMP_RIGHTS, "UsageTerms")
    }
    /// Sets the usage terms `x-default` alternative (`xmpRights:UsageTerms`).
    pub fn set_usage_terms(&mut self, value: &str) {
        self.set_lang_alt(ns::XMP_RIGHTS, "UsageTerms", value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_typed_accessor_round_trips_its_property() {
        // Each accessor must read back exactly what its setter wrote, on the expected property.
        let mut pm = PhotoMetadata::new();
        pm.set_headline("Breaking");
        pm.set_city("Paris");
        pm.set_country("France");
        pm.set_country_code("FRA");
        pm.set_caption("A photo");
        pm.set_copyright_notice("(c) 2024");
        pm.set_title("Sunset");
        pm.set_usage_terms("CC-BY");
        pm.set_keywords(&["sky", "sea"]);
        pm.set_creators(&["Ansel"]);

        assert_eq!(pm.headline(), Some("Breaking"));
        assert_eq!(pm.city(), Some("Paris"));
        assert_eq!(pm.country(), Some("France"));
        assert_eq!(pm.country_code(), Some("FRA"));
        assert_eq!(pm.caption(), Some("A photo"));
        assert_eq!(pm.copyright_notice(), Some("(c) 2024"));
        assert_eq!(pm.title(), Some("Sunset"));
        assert_eq!(pm.usage_terms(), Some("CC-BY"));
        assert_eq!(pm.keywords(), vec!["sky", "sea"]);
        assert_eq!(pm.creators(), vec!["Ansel"]);

        // Each accessor targets a distinct property — ten fields, ten properties.
        assert_eq!(pm.properties.len(), 10);
        // Spot-check the namespaces/names so an accessor can't silently target the wrong property.
        assert!(pm.find(ns::PHOTOSHOP, "Headline").is_some());
        assert!(pm.find(ns::IPTC_CORE, "CountryCode").is_some());
        assert!(pm.find(ns::DC, "rights").is_some());
        assert!(pm.find(ns::XMP_RIGHTS, "UsageTerms").is_some());
    }

    #[test]
    fn simple_accessors_set_and_replace() {
        let mut pm = PhotoMetadata::new();
        assert_eq!(pm.city(), None);
        pm.set_city("Paris");
        assert_eq!(pm.city(), Some("Paris"));
        // Setting again replaces rather than duplicating.
        pm.set_city("Lyon");
        assert_eq!(pm.city(), Some("Lyon"));
        assert_eq!(pm.properties.len(), 1);
    }

    #[test]
    fn lang_alt_reads_x_default_first() {
        let mut pm = PhotoMetadata::new();
        pm.set_caption("A sunset");
        assert_eq!(pm.caption(), Some("A sunset"));
        // A multi-alternative Alt array reads its first (x-default) item.
        pm.properties[0].value = XmpValue::Array(XmpArray::Alt(vec![
            XmpItem::new(XmpValue::Simple("x-default text".to_owned())),
            XmpItem::new(XmpValue::Simple("French text".to_owned())),
        ]));
        assert_eq!(pm.caption(), Some("x-default text"));
        // A plain Simple value (non-conformant but seen in the wild) is also accepted.
        pm.properties[0].value = XmpValue::Simple("plain".to_owned());
        assert_eq!(pm.caption(), Some("plain"));
    }

    #[test]
    fn array_accessors_preserve_order_and_kind() {
        let mut pm = PhotoMetadata::new();
        pm.set_keywords(&["sky", "sea"]);
        pm.set_creators(&["Ansel", "Dorothea"]);
        assert_eq!(pm.keywords(), vec!["sky", "sea"]);
        assert_eq!(pm.creators(), vec!["Ansel", "Dorothea"]);
        // dc:subject is a Bag, dc:creator a Seq.
        let subject = pm.find(ns::DC, "subject").unwrap();
        assert!(matches!(subject.value, XmpValue::Array(XmpArray::Bag(_))));
        let creator = pm.find(ns::DC, "creator").unwrap();
        assert!(matches!(creator.value, XmpValue::Array(XmpArray::Seq(_))));
    }

    #[test]
    fn from_xmp_keeps_only_iptc_namespaces() {
        let meta = XmpMeta {
            properties: vec![
                XmpProperty {
                    namespace: ns::PHOTOSHOP.to_owned(),
                    name: "City".to_owned(),
                    value: XmpValue::Simple("Berlin".to_owned()),
                    qualifiers: Vec::new(),
                },
                XmpProperty {
                    namespace: "http://ns.adobe.com/xap/1.0/".to_owned(),
                    name: "CreatorTool".to_owned(),
                    value: XmpValue::Simple("gamut".to_owned()),
                    qualifiers: Vec::new(),
                },
            ],
        };
        let pm = PhotoMetadata::from_xmp(&meta);
        assert_eq!(pm.properties.len(), 1);
        assert_eq!(pm.city(), Some("Berlin"));
        // Round-trips back out as an XMP graph.
        assert_eq!(pm.to_xmp().properties, pm.properties);
        assert_eq!(PhotoMetadata::from_xmp(&pm.to_xmp()), pm);
    }

    #[test]
    fn get_set_field_round_trip_by_shape() {
        let f = |ns: &'static str, name: &'static str, shape| XmpField { ns, name, shape };
        let mut pm = PhotoMetadata::new();
        pm.set_field(&f(ns::PHOTOSHOP, "Headline", XmpShape::SimpleText), &["H"]);
        pm.set_field(&f(ns::DC, "rights", XmpShape::LangAlt), &["(c)"]);
        pm.set_field(&f(ns::DC, "subject", XmpShape::Bag), &["a", "b"]);
        assert_eq!(
            pm.get_field(&f(ns::PHOTOSHOP, "Headline", XmpShape::SimpleText)),
            vec!["H"]
        );
        assert_eq!(
            pm.get_field(&f(ns::DC, "rights", XmpShape::LangAlt)),
            vec!["(c)"]
        );
        assert_eq!(
            pm.get_field(&f(ns::DC, "subject", XmpShape::Bag)),
            vec!["a", "b"]
        );
        // Absent field yields an empty list; empty value list leaves a scalar unset.
        assert!(
            pm.get_field(&f(ns::PHOTOSHOP, "City", XmpShape::SimpleText))
                .is_empty()
        );
        pm.set_field(&f(ns::PHOTOSHOP, "City", XmpShape::SimpleText), &[]);
        assert_eq!(pm.city(), None);

        // set_field must store the RDF container kind that matches the shape.
        assert!(matches!(
            pm.find(ns::DC, "subject").unwrap().value,
            XmpValue::Array(XmpArray::Bag(_))
        ));
        pm.set_field(&f(ns::DC, "creator", XmpShape::Seq), &["x"]);
        assert!(matches!(
            pm.find(ns::DC, "creator").unwrap().value,
            XmpValue::Array(XmpArray::Seq(_))
        ));
    }
}
