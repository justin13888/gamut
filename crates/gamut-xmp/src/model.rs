//! The XMP property graph data model (Adobe XMP Part 1 §6).
//!
//! XMP describes one resource as a set of *properties*. Each [`XmpProperty`] is a namespaced name
//! with an [`XmpValue`] — a simple literal, a nested structure, or an array — and an optional list
//! of qualifiers (Part 1 §6.4). Arrays hold [`XmpItem`]s so an item can carry its own qualifiers
//! (notably the `xml:lang` of a language alternative, Part 1 §8.2.2.4).
//!
//! The accessors on [`XmpMeta`] (`get_text`, `set_text`, `get_lang_alt`, `set_lang_alt`, …) cover
//! the common cases without walking the tree by hand; the public fields stay available for the rest.

use crate::namespace::XML_NAMESPACE;

/// A parsed XMP packet's metadata: a set of top-level properties.
///
/// XMP is, at heart, a set of (namespace, name) → value triples describing one resource. Nested
/// structure and ordering are carried in the [`XmpValue`] tree, not here. Property order is
/// preserved (it is the document order on read, and the order the serializer emits).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct XmpMeta {
    /// The top-level properties, each qualified by its namespace.
    pub properties: Vec<XmpProperty>,
}

/// One XMP property: a namespaced name, its value, and any qualifiers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmpProperty {
    /// The XML namespace URI the property name lives in (e.g. the Dublin Core URI for `dc:title`).
    pub namespace: String,
    /// The local property name (e.g. `title`).
    pub name: String,
    /// The property's value.
    pub value: XmpValue,
    /// Qualifiers attached to the value (e.g. `xml:lang` on a language alternative, or an
    /// arbitrary RDF qualifier). Empty for a plain property.
    pub qualifiers: Vec<XmpProperty>,
}

/// An XMP value: a simple literal, a URI, a nested structure, or an array (Adobe XMP Part 1 §6.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XmpValue {
    /// A simple literal value (text; typed values like dates/integers are text in the model,
    /// Part 1 §8.2).
    Simple(String),
    /// A simple *URI* value — serialized with `rdf:resource` rather than as element text
    /// (Part 1 §6.3.2 / §7.5). Kept distinct from [`XmpValue::Simple`] so the distinction
    /// round-trips.
    Uri(String),
    /// A structured value — an unordered set of named fields, themselves properties
    /// (Part 1 §6.3.3).
    Structured(Vec<XmpProperty>),
    /// An array value — see [`XmpArray`] for the RDF container kind.
    Array(XmpArray),
}

impl Default for XmpValue {
    /// An empty simple value — the neutral default for a freshly-declared property.
    fn default() -> Self {
        XmpValue::Simple(String::new())
    }
}

/// The three RDF array kinds XMP uses (Adobe XMP Part 1 §6.3.4). Items are 1-based and should share
/// a value type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XmpArray {
    /// `rdf:Bag` — an unordered array.
    Bag(Vec<XmpItem>),
    /// `rdf:Seq` — an ordered array.
    Seq(Vec<XmpItem>),
    /// `rdf:Alt` — an array of alternatives; the common case is language alternatives selected by
    /// the `xml:lang` qualifier, with the `"x-default"` item first (Part 1 §8.2.2.4).
    Alt(Vec<XmpItem>),
}

impl XmpArray {
    /// The array's items, regardless of kind.
    #[must_use]
    pub fn items(&self) -> &[XmpItem] {
        match self {
            XmpArray::Bag(i) | XmpArray::Seq(i) | XmpArray::Alt(i) => i,
        }
    }
}

/// One item of an [`XmpArray`]: a value plus any qualifiers on that item.
///
/// An `rdf:li` may carry qualifiers — most importantly the `xml:lang` of a language alternative
/// (Part 1 §7.7, §8.2.2.4) — so array items are richer than a bare [`XmpValue`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmpItem {
    /// The item's value.
    pub value: XmpValue,
    /// Qualifiers on the item (e.g. `xml:lang`). Empty for a plain item.
    pub qualifiers: Vec<XmpProperty>,
}

impl XmpProperty {
    /// Creates a property with no qualifiers.
    #[must_use]
    pub fn new(namespace: impl Into<String>, name: impl Into<String>, value: XmpValue) -> Self {
        Self {
            namespace: namespace.into(),
            name: name.into(),
            value,
            qualifiers: Vec::new(),
        }
    }

    /// The text of this property's value if it is simple ([`XmpValue::Simple`] or
    /// [`XmpValue::Uri`]), otherwise `None`.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.value.text()
    }

    /// The value of this property's `xml:lang` qualifier, if any (Part 1 §6.4).
    #[must_use]
    pub fn lang(&self) -> Option<&str> {
        lang_of(&self.qualifiers)
    }
}

impl XmpItem {
    /// Creates an array item with no qualifiers.
    #[must_use]
    pub fn new(value: XmpValue) -> Self {
        Self {
            value,
            qualifiers: Vec::new(),
        }
    }

    /// Creates a text item tagged with an `xml:lang` qualifier — the shape of a language
    /// alternative entry (Part 1 §8.2.2.4).
    #[must_use]
    pub fn lang_text(lang: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            value: XmpValue::Simple(text.into()),
            qualifiers: vec![XmpProperty::new(
                XML_NAMESPACE,
                "lang",
                XmpValue::Simple(lang.into()),
            )],
        }
    }

    /// The value of this item's `xml:lang` qualifier, if any.
    #[must_use]
    pub fn lang(&self) -> Option<&str> {
        lang_of(&self.qualifiers)
    }

    /// The text of this item's value if it is simple ([`XmpValue::Simple`] or [`XmpValue::Uri`]),
    /// otherwise `None`.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        self.value.text()
    }
}

impl XmpValue {
    /// The text of this value if it is simple ([`XmpValue::Simple`] or [`XmpValue::Uri`]),
    /// otherwise `None`.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            XmpValue::Simple(s) | XmpValue::Uri(s) => Some(s),
            _ => None,
        }
    }
}

/// The `xml:lang` qualifier value within a qualifier list, if present.
fn lang_of(qualifiers: &[XmpProperty]) -> Option<&str> {
    qualifiers
        .iter()
        .find(|q| q.namespace == XML_NAMESPACE && q.name == "lang")
        .and_then(XmpProperty::text)
}

impl XmpMeta {
    /// An empty metadata set.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The top-level property with the given namespace URI and local name, if present.
    #[must_use]
    pub fn get(&self, namespace: &str, name: &str) -> Option<&XmpProperty> {
        self.properties
            .iter()
            .find(|p| p.namespace == namespace && p.name == name)
    }

    /// A mutable reference to the top-level property with the given namespace URI and local name.
    #[must_use]
    pub fn get_mut(&mut self, namespace: &str, name: &str) -> Option<&mut XmpProperty> {
        self.properties
            .iter_mut()
            .find(|p| p.namespace == namespace && p.name == name)
    }

    /// The text of a simple property (Part 1 §8.2), or `None` if it is absent or not simple.
    #[must_use]
    pub fn get_text(&self, namespace: &str, name: &str) -> Option<&str> {
        self.get(namespace, name)?.text()
    }

    /// The text of a language alternative (a `dc:title`-style `rdf:Alt`, Part 1 §8.2.2.4) for the
    /// given language tag, or `None` if the property is absent, not a language alternative, or has
    /// no matching item.
    ///
    /// Language matching is case-insensitive (Part 1 §8.2.2.4 / RFC 3066). Pass `"x-default"` for
    /// the default item.
    #[must_use]
    pub fn get_lang_alt(&self, namespace: &str, name: &str, lang: &str) -> Option<&str> {
        let XmpValue::Array(XmpArray::Alt(items)) = &self.get(namespace, name)?.value else {
            return None;
        };
        items
            .iter()
            .find(|item| item.lang().is_some_and(|l| l.eq_ignore_ascii_case(lang)))
            .and_then(XmpItem::text)?
            .into()
    }

    /// Inserts a property, replacing any existing top-level property with the same namespace URI
    /// and local name (so a name maps to at most one value).
    pub fn set(&mut self, property: XmpProperty) {
        if let Some(existing) = self.get_mut(&property.namespace, &property.name) {
            *existing = property;
        } else {
            self.properties.push(property);
        }
    }

    /// Sets a simple text property, replacing any existing property of the same name.
    pub fn set_text(
        &mut self,
        namespace: impl Into<String>,
        name: impl Into<String>,
        value: impl Into<String>,
    ) {
        self.set(XmpProperty::new(
            namespace,
            name,
            XmpValue::Simple(value.into()),
        ));
    }

    /// Sets one entry of a language alternative (`rdf:Alt`, Part 1 §8.2.2.4), creating the property
    /// if needed.
    ///
    /// An existing entry for the same language (case-insensitive) is updated in place; otherwise a
    /// new entry is added. The `"x-default"` entry is kept first, as the spec requires. If the
    /// property exists but is not a language alternative it is replaced by one.
    pub fn set_lang_alt(
        &mut self,
        namespace: impl Into<String>,
        name: impl Into<String>,
        lang: impl Into<String>,
        value: impl Into<String>,
    ) {
        let namespace = namespace.into();
        let name = name.into();
        let lang = lang.into();
        let value = value.into();

        let items = match self.get_mut(&namespace, &name).map(|p| &mut p.value) {
            Some(XmpValue::Array(XmpArray::Alt(items))) => items,
            _ => {
                self.set(XmpProperty::new(
                    &namespace,
                    &name,
                    XmpValue::Array(XmpArray::Alt(Vec::new())),
                ));
                let Some(XmpValue::Array(XmpArray::Alt(items))) =
                    self.get_mut(&namespace, &name).map(|p| &mut p.value)
                else {
                    return; // unreachable: just set it
                };
                items
            }
        };

        if let Some(item) = items
            .iter_mut()
            .find(|item| item.lang().is_some_and(|l| l.eq_ignore_ascii_case(&lang)))
        {
            item.value = XmpValue::Simple(value);
        } else if lang.eq_ignore_ascii_case("x-default") {
            items.insert(0, XmpItem::lang_text(lang, value));
        } else {
            items.push(XmpItem::lang_text(lang, value));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DC: &str = "http://purl.org/dc/elements/1.1/";

    #[test]
    fn get_set_text_round_trips_and_replaces() {
        let mut meta = XmpMeta::new();
        assert_eq!(meta.get_text(DC, "creator"), None);
        meta.set_text(DC, "creator", "Ada");
        assert_eq!(meta.get_text(DC, "creator"), Some("Ada"));
        // set replaces rather than duplicating.
        meta.set_text(DC, "creator", "Grace");
        assert_eq!(meta.get_text(DC, "creator"), Some("Grace"));
        assert_eq!(meta.properties.len(), 1);
    }

    #[test]
    fn get_text_rejects_non_simple_values() {
        let mut meta = XmpMeta::new();
        meta.set(XmpProperty::new(
            DC,
            "subject",
            XmpValue::Array(XmpArray::Bag(vec![XmpItem::new(XmpValue::Simple(
                "x".into(),
            ))])),
        ));
        assert_eq!(meta.get_text(DC, "subject"), None);
    }

    #[test]
    fn uri_value_is_simple_text_too() {
        let mut meta = XmpMeta::new();
        meta.set(XmpProperty::new(
            "http://ns.adobe.com/xap/1.0/",
            "BaseURL",
            XmpValue::Uri("http://example.com/".into()),
        ));
        assert_eq!(
            meta.get_text("http://ns.adobe.com/xap/1.0/", "BaseURL"),
            Some("http://example.com/")
        );
    }

    #[test]
    fn lang_alt_keeps_x_default_first_and_matches_case_insensitively() {
        let mut meta = XmpMeta::new();
        meta.set_lang_alt(DC, "title", "en-US", "Hello");
        meta.set_lang_alt(DC, "title", "fr-FR", "Bonjour");
        meta.set_lang_alt(DC, "title", "x-default", "Hello");

        // x-default inserted first despite being added last.
        let XmpValue::Array(XmpArray::Alt(items)) = &meta.get(DC, "title").unwrap().value else {
            panic!("expected Alt");
        };
        assert_eq!(items[0].lang(), Some("x-default"));
        assert_eq!(items.len(), 3);

        // case-insensitive lookup (RFC 3066).
        assert_eq!(meta.get_lang_alt(DC, "title", "EN-us"), Some("Hello"));
        assert_eq!(meta.get_lang_alt(DC, "title", "fr-fr"), Some("Bonjour"));
        assert_eq!(meta.get_lang_alt(DC, "title", "de"), None);
    }

    #[test]
    fn set_lang_alt_updates_existing_entry_in_place() {
        let mut meta = XmpMeta::new();
        meta.set_lang_alt(DC, "title", "en", "first");
        meta.set_lang_alt(DC, "title", "EN", "second");
        assert_eq!(meta.get_lang_alt(DC, "title", "en"), Some("second"));
        let XmpValue::Array(XmpArray::Alt(items)) = &meta.get(DC, "title").unwrap().value else {
            panic!("expected Alt");
        };
        assert_eq!(items.len(), 1, "same language must not duplicate");
    }

    #[test]
    fn get_lang_alt_returns_none_for_non_alt() {
        let mut meta = XmpMeta::new();
        meta.set_text(DC, "title", "plain");
        assert_eq!(meta.get_lang_alt(DC, "title", "x-default"), None);
    }

    #[test]
    fn default_value_is_empty_simple() {
        assert_eq!(XmpValue::default(), XmpValue::Simple(String::new()));
    }

    #[test]
    fn array_items_accessor_covers_every_kind() {
        let one = vec![XmpItem::new(XmpValue::Simple("a".into()))];
        assert_eq!(XmpArray::Bag(one.clone()).items().len(), 1);
        assert_eq!(XmpArray::Seq(one.clone()).items().len(), 1);
        assert_eq!(XmpArray::Alt(one).items().len(), 1);
    }
}
