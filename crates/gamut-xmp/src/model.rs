//! The XMP property graph data model (Adobe XMP Part 1 §6).

/// A parsed XMP packet's metadata: a set of top-level properties.
///
/// XMP is, at heart, a set of (namespace, name) → value triples describing one resource. Nested
/// structure and ordering are carried in the [`XmpValue`] tree, not here.
pub struct XmpMeta {
    /// The top-level properties, each qualified by its namespace.
    pub properties: Vec<XmpProperty>,
}

/// One XMP property: a namespaced name, its value, and any qualifiers.
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

/// An XMP value: a simple literal, a nested structure, or an array (Adobe XMP Part 1 §6.4–6.5).
pub enum XmpValue {
    /// A simple literal value (text; typed values like dates/integers are text in the model).
    Simple(String),
    /// A structured value — an unordered set of named fields, themselves properties.
    Structured(Vec<XmpProperty>),
    /// An array value — see [`XmpArray`] for the RDF container kind.
    Array(XmpArray),
}

/// The three RDF array kinds XMP uses (Adobe XMP Part 1 §6.5).
pub enum XmpArray {
    /// `rdf:Bag` — an unordered array.
    Bag(Vec<XmpValue>),
    /// `rdf:Seq` — an ordered array.
    Seq(Vec<XmpValue>),
    /// `rdf:Alt` — an array of alternatives; the common case is language alternatives selected by
    /// the `xml:lang` qualifier.
    Alt(Vec<XmpValue>),
}
