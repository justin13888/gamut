//! The XMP writer: an [`XmpMeta`] graph → **canonical RDF/XML** (Adobe XMP Part 1 §7).
//!
//! RDF admits many serializations of one graph; the canonical form fixes a single shape so output
//! is stable, diffable, and round-trippable. This serializer locks every choice §7.9 leaves open:
//!
//! - one `<rdf:Description rdf:about="">` holding all properties;
//! - simple values as element text, URI values via `rdf:resource`, structures as a nested
//!   `rdf:Description`, arrays as `rdf:Bag`/`Seq`/`Alt` of `rdf:li` (never the attribute shorthands,
//!   `rdf:parseType="Resource"`, or `rdf:_n` items);
//! - `xml:lang` as an attribute, other qualifiers via the `rdf:value` form (§7.8);
//! - all `xmlns` declarations on `rdf:RDF`, ordered `rdf` first then alphabetically by prefix, with
//!   unknown namespaces given stable `ns1`, `ns2`, … prefixes;
//! - one-space-per-level indentation, UTF-8, no byte-order mark.
//!
//! [`XmpMeta::to_packet`] / [`XmpMeta::to_rdf`] are the shortcuts; [`XmpWriter`] exposes the knobs
//! (the `x:xmpmeta` wrapper, writability, and padding).

use crate::model::{XmpArray, XmpItem, XmpMeta, XmpProperty, XmpValue};
use crate::namespace::{RDF_NAMESPACE, WellKnownNs, XML_NAMESPACE, XMPMETA_NAMESPACE};

/// The fixed packet identifier Adobe specifies for the `<?xpacket?>` header (Part 1 §7.3.2).
const XPACKET_ID: &str = "W5M0MpCehiHzreSzNTczkc9d";

/// Default trailing padding for a writable packet, in bytes (Part 1 §7.3.2 suggests ~2 KB).
const DEFAULT_PADDING: usize = 2048;

/// A configurable serializer for [`XmpMeta`] → canonical RDF/XML.
///
/// Construct with [`XmpWriter::new`], adjust with the builder methods, then call
/// [`XmpWriter::serialize`] for a full `<?xpacket?>` packet or [`XmpWriter::serialize_body`] for the
/// RDF/XML alone. For the common cases reach for [`XmpMeta::to_packet`] / [`XmpMeta::to_rdf`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmpWriter {
    /// Whether to wrap `rdf:RDF` in the optional `x:xmpmeta` element (Part 1 §7.3.3).
    wrap_xmpmeta: bool,
    /// Whether the packet trailer says `end="w"` (in-place editable) and carries padding.
    writable: bool,
    /// Trailing padding for a writable packet, in bytes.
    padding: usize,
}

impl Default for XmpWriter {
    fn default() -> Self {
        Self {
            wrap_xmpmeta: true,
            writable: true,
            padding: DEFAULT_PADDING,
        }
    }
}

impl XmpWriter {
    /// A writer with the default settings: `x:xmpmeta` wrapper, writable, ~2 KB padding.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets whether the body is wrapped in the optional `x:xmpmeta` element.
    #[must_use]
    pub fn wrap_xmpmeta(mut self, wrap: bool) -> Self {
        self.wrap_xmpmeta = wrap;
        self
    }

    /// Sets whether the packet is writable in place (`end="w"` with padding) or read-only
    /// (`end="r"`, no padding).
    #[must_use]
    pub fn writable(mut self, writable: bool) -> Self {
        self.writable = writable;
        self
    }

    /// Sets the trailing padding, in bytes, for a writable packet. Ignored when read-only.
    #[must_use]
    pub fn padding(mut self, bytes: usize) -> Self {
        self.padding = bytes;
        self
    }

    /// Serializes the canonical RDF/XML body (no `<?xpacket?>` wrapper).
    #[must_use]
    pub fn serialize_body(&self, meta: &XmpMeta) -> String {
        let ns = NsMap::gather(meta);
        let mut out = String::new();

        let rdf_level = if self.wrap_xmpmeta {
            out.push_str("<x:xmpmeta xmlns:x=\"");
            out.push_str(XMPMETA_NAMESPACE);
            out.push_str("\">\n");
            1
        } else {
            0
        };

        indent(&mut out, rdf_level);
        out.push_str("<rdf:RDF");
        out.push_str(&ns.declarations());
        out.push_str(">\n");

        let desc_level = rdf_level + 1;
        indent(&mut out, desc_level);
        if meta.properties.is_empty() {
            out.push_str("<rdf:Description rdf:about=\"\"/>\n");
        } else {
            out.push_str("<rdf:Description rdf:about=\"\">\n");
            for property in &meta.properties {
                emit_property(&mut out, desc_level + 1, property, &ns);
            }
            indent(&mut out, desc_level);
            out.push_str("</rdf:Description>\n");
        }

        indent(&mut out, rdf_level);
        out.push_str("</rdf:RDF>");
        if self.wrap_xmpmeta {
            out.push_str("\n</x:xmpmeta>");
        }
        out
    }

    /// Serializes a full XMP packet: the `<?xpacket?>` header, the canonical body, padding (if
    /// writable), and the trailer. UTF-8, no byte-order mark.
    #[must_use]
    pub fn serialize(&self, meta: &XmpMeta) -> Vec<u8> {
        let mut out = String::new();
        out.push_str("<?xpacket begin=\"\" id=\"");
        out.push_str(XPACKET_ID);
        out.push_str("\"?>\n");
        out.push_str(&self.serialize_body(meta));
        out.push('\n');
        if self.writable {
            // Whitespace padding, a newline every 100 bytes for display (Part 1 §7.3.2).
            for i in 0..self.padding {
                out.push(if (i + 1) % 100 == 0 { '\n' } else { ' ' });
            }
            out.push('\n');
        }
        out.push_str("<?xpacket end=\"");
        out.push(if self.writable { 'w' } else { 'r' });
        out.push_str("\"?>");
        out.into_bytes()
    }
}

impl XmpMeta {
    /// Serializes to a full XMP packet with the default settings (`x:xmpmeta` wrapper, writable,
    /// padded) — the inverse of [`XmpMeta::from_packet`].
    #[must_use]
    pub fn to_packet(&self) -> Vec<u8> {
        XmpWriter::new().serialize(self)
    }

    /// Serializes to the canonical RDF/XML body, without the `<?xpacket?>` wrapper.
    #[must_use]
    pub fn to_rdf(&self) -> String {
        XmpWriter::new().serialize_body(self)
    }
}

// ---------------------------------------------------------------------------------------------------
// Namespace gathering and prefix assignment.
// ---------------------------------------------------------------------------------------------------

/// Maps the data namespaces used in a graph to serialization prefixes (well-known, else `nsN`).
struct NsMap {
    /// `(uri, prefix)` for each data namespace, in first-encounter order.
    entries: Vec<(String, String)>,
}

impl NsMap {
    /// Walks `meta`, collecting every data namespace and assigning it a prefix.
    fn gather(meta: &XmpMeta) -> NsMap {
        let mut map = NsMap {
            entries: Vec::new(),
        };
        for property in &meta.properties {
            map.visit_property(property);
        }
        map
    }

    fn visit_property(&mut self, property: &XmpProperty) {
        self.add(&property.namespace);
        self.visit_value(&property.value);
        for qualifier in &property.qualifiers {
            self.visit_property(qualifier);
        }
    }

    fn visit_value(&mut self, value: &XmpValue) {
        match value {
            XmpValue::Simple(_) | XmpValue::Uri(_) => {}
            XmpValue::Structured(fields) => {
                for field in fields {
                    self.visit_property(field);
                }
            }
            XmpValue::Array(array) => {
                for item in array.items() {
                    self.visit_value(&item.value);
                    for qualifier in &item.qualifiers {
                        self.visit_property(qualifier);
                    }
                }
            }
        }
    }

    /// Records a namespace URI, assigning it a prefix. `rdf:` and `xml:` are handled implicitly and
    /// never enter the map.
    fn add(&mut self, uri: &str) {
        if uri == RDF_NAMESPACE || uri == XML_NAMESPACE {
            return;
        }
        if self.entries.iter().any(|(u, _)| u == uri) {
            return;
        }
        let prefix = WellKnownNs::from_uri(uri).map_or_else(
            || {
                let n = self
                    .entries
                    .iter()
                    .filter(|(u, _)| WellKnownNs::from_uri(u).is_none())
                    .count();
                format!("ns{}", n + 1)
            },
            |well_known| well_known.prefix().to_owned(),
        );
        self.entries.push((uri.to_owned(), prefix));
    }

    /// The serialization prefix for a namespace URI.
    fn prefix_for(&self, uri: &str) -> &str {
        if uri == RDF_NAMESPACE {
            return "rdf";
        }
        if uri == XML_NAMESPACE {
            return "xml";
        }
        self.entries
            .iter()
            .find(|(u, _)| u == uri)
            .map_or("rdf", |(_, prefix)| prefix.as_str())
    }

    /// The `xmlns` declarations for `rdf:RDF`: `rdf` first, then data namespaces by prefix.
    fn declarations(&self) -> String {
        let mut out = String::new();
        out.push_str(" xmlns:rdf=\"");
        out.push_str(RDF_NAMESPACE);
        out.push('"');

        let mut data: Vec<&(String, String)> = self.entries.iter().collect();
        data.sort_by(|a, b| a.1.cmp(&b.1));
        for (uri, prefix) in data {
            out.push_str(" xmlns:");
            out.push_str(prefix);
            out.push_str("=\"");
            push_escaped_attr(&mut out, uri);
            out.push('"');
        }
        out
    }
}

// ---------------------------------------------------------------------------------------------------
// Emission.
// ---------------------------------------------------------------------------------------------------

/// Emits a property element `<prefix:name …>`.
fn emit_property(out: &mut String, level: usize, property: &XmpProperty, ns: &NsMap) {
    let tag = format!("{}:{}", ns.prefix_for(&property.namespace), property.name);
    emit_node(out, level, &tag, &property.value, &property.qualifiers, ns);
}

/// Emits an element named `tag` for a value and its qualifiers: `xml:lang` becomes an attribute, and
/// any other qualifier triggers the `rdf:value` form (Part 1 §7.8).
fn emit_node(
    out: &mut String,
    level: usize,
    tag: &str,
    value: &XmpValue,
    qualifiers: &[XmpProperty],
    ns: &NsMap,
) {
    let lang = lang_of(qualifiers);
    let general: Vec<&XmpProperty> = qualifiers.iter().filter(|q| !is_lang(q)).collect();

    if general.is_empty() {
        emit_value_element(out, level, tag, lang, value, ns);
        return;
    }

    indent(out, level);
    out.push('<');
    out.push_str(tag);
    push_lang(out, lang);
    out.push_str(">\n");
    indent(out, level + 1);
    out.push_str("<rdf:Description>\n");
    emit_value_element(out, level + 2, "rdf:value", None, value, ns);
    for qualifier in general {
        emit_property(out, level + 2, qualifier, ns);
    }
    indent(out, level + 1);
    out.push_str("</rdf:Description>\n");
    indent(out, level);
    out.push_str("</");
    out.push_str(tag);
    out.push_str(">\n");
}

/// Emits an element named `tag` for a value (no general qualifiers), with an optional `xml:lang`.
fn emit_value_element(
    out: &mut String,
    level: usize,
    tag: &str,
    lang: Option<&str>,
    value: &XmpValue,
    ns: &NsMap,
) {
    indent(out, level);
    match value {
        XmpValue::Simple(text) => {
            out.push('<');
            out.push_str(tag);
            push_lang(out, lang);
            out.push('>');
            push_escaped_text(out, text);
            out.push_str("</");
            out.push_str(tag);
            out.push_str(">\n");
        }
        XmpValue::Uri(uri) => {
            out.push('<');
            out.push_str(tag);
            push_lang(out, lang);
            out.push_str(" rdf:resource=\"");
            push_escaped_attr(out, uri);
            out.push_str("\"/>\n");
        }
        XmpValue::Structured(fields) => {
            out.push('<');
            out.push_str(tag);
            push_lang(out, lang);
            out.push_str(">\n");
            indent(out, level + 1);
            out.push_str("<rdf:Description>\n");
            for field in fields {
                emit_property(out, level + 2, field, ns);
            }
            indent(out, level + 1);
            out.push_str("</rdf:Description>\n");
            indent(out, level);
            out.push_str("</");
            out.push_str(tag);
            out.push_str(">\n");
        }
        XmpValue::Array(array) => {
            let (container, items) = array_parts(array);
            out.push('<');
            out.push_str(tag);
            push_lang(out, lang);
            out.push_str(">\n");
            indent(out, level + 1);
            out.push('<');
            out.push_str(container);
            out.push_str(">\n");
            for item in items {
                emit_node(out, level + 2, "rdf:li", &item.value, &item.qualifiers, ns);
            }
            indent(out, level + 1);
            out.push_str("</");
            out.push_str(container);
            out.push_str(">\n");
            indent(out, level);
            out.push_str("</");
            out.push_str(tag);
            out.push_str(">\n");
        }
    }
}

/// The container element name and items of an array.
fn array_parts(array: &XmpArray) -> (&'static str, &[XmpItem]) {
    match array {
        XmpArray::Bag(items) => ("rdf:Bag", items),
        XmpArray::Seq(items) => ("rdf:Seq", items),
        XmpArray::Alt(items) => ("rdf:Alt", items),
    }
}

/// Pushes ` xml:lang="…"` when a language is present.
fn push_lang(out: &mut String, lang: Option<&str>) {
    if let Some(lang) = lang {
        out.push_str(" xml:lang=\"");
        push_escaped_attr(out, lang);
        out.push('"');
    }
}

/// Whether a qualifier is `xml:lang`.
fn is_lang(qualifier: &XmpProperty) -> bool {
    qualifier.namespace == XML_NAMESPACE && qualifier.name == "lang"
}

/// The `xml:lang` value among a qualifier list, if any.
fn lang_of(qualifiers: &[XmpProperty]) -> Option<&str> {
    qualifiers
        .iter()
        .find(|q| is_lang(q))
        .and_then(XmpProperty::text)
}

/// Pushes `count` indentation spaces (one per nesting level).
fn indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push(' ');
    }
}

/// Appends `text` with the markup-significant characters escaped (Part 1 §7.5; never CDATA).
fn push_escaped_text(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
}

/// Appends an attribute value with markup characters and the double quote escaped.
fn push_escaped_attr(out: &mut String, text: &str) {
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DC: &str = "http://purl.org/dc/elements/1.1/";
    const XMP: &str = "http://ns.adobe.com/xap/1.0/";

    /// Serializes a graph of the given properties to a bare `rdf:RDF` body (no wrapper) for compact,
    /// byte-exact assertions.
    fn body(properties: Vec<XmpProperty>) -> String {
        XmpWriter::new()
            .wrap_xmpmeta(false)
            .serialize_body(&XmpMeta { properties })
    }

    #[test]
    fn simple_value_is_element_text() {
        let out = body(vec![XmpProperty::new(
            XMP,
            "Rating",
            XmpValue::Simple("3".into()),
        )]);
        assert_eq!(
            out,
            "<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\" \
             xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\">\n \
             <rdf:Description rdf:about=\"\">\n  \
             <xmp:Rating>3</xmp:Rating>\n \
             </rdf:Description>\n\
             </rdf:RDF>"
        );
    }

    #[test]
    fn uri_value_uses_rdf_resource() {
        let out = body(vec![XmpProperty::new(
            XMP,
            "BaseURL",
            XmpValue::Uri("http://example.com/".into()),
        )]);
        assert!(out.contains("<xmp:BaseURL rdf:resource=\"http://example.com/\"/>"));
    }

    #[test]
    fn bag_emits_rdf_li_in_order() {
        let out = body(vec![XmpProperty::new(
            DC,
            "subject",
            XmpValue::Array(XmpArray::Bag(vec![
                XmpItem::new(XmpValue::Simple("a".into())),
                XmpItem::new(XmpValue::Simple("b".into())),
            ])),
        )]);
        let expected = "  <dc:subject>\n   <rdf:Bag>\n    <rdf:li>a</rdf:li>\n    \
                        <rdf:li>b</rdf:li>\n   </rdf:Bag>\n  </dc:subject>";
        assert!(out.contains(expected), "got:\n{out}");
    }

    #[test]
    fn language_alternative_uses_xml_lang_attribute() {
        let mut meta = XmpMeta::new();
        meta.set_lang_alt(DC, "title", "x-default", "Hi");
        meta.set_lang_alt(DC, "title", "fr", "Salut");
        let out = XmpWriter::new().wrap_xmpmeta(false).serialize_body(&meta);
        assert!(out.contains("<rdf:Alt>"));
        assert!(out.contains("<rdf:li xml:lang=\"x-default\">Hi</rdf:li>"));
        assert!(out.contains("<rdf:li xml:lang=\"fr\">Salut</rdf:li>"));
    }

    #[test]
    fn structure_uses_nested_description() {
        let out = body(vec![XmpProperty::new(
            XMP,
            "Thumb",
            XmpValue::Structured(vec![XmpProperty::new(
                XMP,
                "w",
                XmpValue::Simple("9".into()),
            )]),
        )]);
        let expected = "  <xmp:Thumb>\n   <rdf:Description>\n    <xmp:w>9</xmp:w>\n   \
                        </rdf:Description>\n  </xmp:Thumb>";
        assert!(out.contains(expected), "got:\n{out}");
    }

    #[test]
    fn general_qualifier_uses_rdf_value_form() {
        let prop = XmpProperty {
            namespace: DC.into(),
            name: "rights".into(),
            value: XmpValue::Simple("(c) Me".into()),
            qualifiers: vec![XmpProperty::new(
                XMP,
                "owner",
                XmpValue::Simple("Me".into()),
            )],
        };
        let out = body(vec![prop]);
        let expected = "  <dc:rights>\n   <rdf:Description>\n    <rdf:value>(c) Me</rdf:value>\n    \
                        <xmp:owner>Me</xmp:owner>\n   </rdf:Description>\n  </dc:rights>";
        assert!(out.contains(expected), "got:\n{out}");
    }

    #[test]
    fn text_and_attributes_are_escaped() {
        let out = body(vec![XmpProperty::new(
            DC,
            "rights",
            XmpValue::Simple("a < b & c > d".into()),
        )]);
        assert!(out.contains("<dc:rights>a &lt; b &amp; c &gt; d</dc:rights>"));

        let mut meta = XmpMeta::new();
        meta.set_lang_alt(DC, "title", "x-\"q\"", "v");
        let out = meta.to_rdf();
        assert!(out.contains("xml:lang=\"x-&quot;q&quot;\""), "got:\n{out}");
    }

    #[test]
    fn namespaces_are_declared_rdf_first_then_alphabetical_with_synthesized() {
        // Two well-known (dc, xmp) plus one unknown → ns1; declared rdf, then by prefix.
        let out = body(vec![
            XmpProperty::new(XMP, "Rating", XmpValue::Simple("1".into())),
            XmpProperty::new(DC, "format", XmpValue::Simple("text/plain".into())),
            XmpProperty::new("http://example.com/x/", "k", XmpValue::Simple("v".into())),
        ]);
        // Without the x:xmpmeta wrapper the first line is the rdf:RDF element with all decls.
        let header = out.lines().next().unwrap();
        assert_eq!(
            header,
            "<rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\" \
             xmlns:dc=\"http://purl.org/dc/elements/1.1/\" \
             xmlns:ns1=\"http://example.com/x/\" \
             xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\">"
        );
        assert!(out.contains("<ns1:k>v</ns1:k>"));
    }

    #[test]
    fn empty_meta_self_closes_description() {
        let out = body(vec![]);
        assert!(
            out.contains("<rdf:Description rdf:about=\"\"/>"),
            "got:\n{out}"
        );
    }

    #[test]
    fn packet_has_header_body_and_trailer() {
        let meta = XmpMeta {
            properties: vec![XmpProperty::new(
                XMP,
                "Rating",
                XmpValue::Simple("3".into()),
            )],
        };
        let packet = String::from_utf8(meta.to_packet()).unwrap();
        assert!(packet.starts_with("<?xpacket begin=\"\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n"));
        assert!(packet.contains("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">"));
        assert!(packet.ends_with("<?xpacket end=\"w\"?>"));
        assert!(packet.contains("<xmp:Rating>3</xmp:Rating>"));
    }

    #[test]
    fn read_only_writer_omits_padding() {
        let writer = XmpWriter::new().writable(false);
        let packet = String::from_utf8(writer.serialize(&XmpMeta::new())).unwrap();
        assert!(packet.ends_with("<?xpacket end=\"r\"?>"));
        // No run of padding spaces between the body and the trailer.
        assert!(!packet.contains("   <?xpacket end"));
    }

    #[test]
    fn writable_writer_emits_requested_padding() {
        let padded = XmpWriter::new()
            .padding(2000)
            .serialize(&XmpMeta::new())
            .len();
        let bare = XmpWriter::new()
            .writable(false)
            .serialize(&XmpMeta::new())
            .len();
        // The writable packet is larger by at least the requested padding.
        assert!(padded >= bare + 2000, "padded {padded} vs bare {bare}");
    }
}
