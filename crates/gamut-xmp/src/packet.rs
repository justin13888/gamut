//! The XMP packet wrapper.

/// The serialized form of an XMP packet — the RDF/XML body inside its `<?xpacket?>` wrapper.
///
/// XMP is embedded as an `<?xpacket begin=… id=…?>` processing instruction, the `x:xmpmeta` /
/// `rdf:RDF` body, then `<?xpacket end='r'|'w'?>`. A writable (`'w'`) packet carries trailing
/// whitespace padding so it can be edited in place without rewriting the whole file — a detail the
/// serializer must reproduce.
pub struct XmpPacket {
    /// The RDF/XML body between the opening and closing `xpacket` instructions.
    pub body: String,
    /// Whether the packet is writable in place (`end='w'`) versus read-only (`end='r'`).
    pub writable: bool,
    /// The number of trailing padding bytes reserved for in-place edits.
    pub padding: usize,
}
