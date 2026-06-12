//! The XMP writer.

/// Writer for an XMP packet.
///
/// Will serialise an [`crate::XmpMeta`] to **canonical RDF/XML** (Adobe XMP Part 1 §7) inside an
/// `<?xpacket?>` wrapper — the crate's **keystone**, since canonical form fixes element vs.
/// attribute serialization, namespace declaration placement, and array/struct nesting so output is
/// stable and round-trippable. Implementation pending (see issue #34).
pub struct XmpWriter;
