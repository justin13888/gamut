//! The XMP reader.

/// Reader for an XMP packet.
///
/// Will parse the `<?xpacket?>`-wrapped RDF/XML into an [`crate::XmpMeta`] property graph,
/// resolving namespaces, structured/array values, qualifiers, and language alternatives.
/// Implementation pending (see issue #34).
pub struct XmpReader;
