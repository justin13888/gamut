//! The crate's error type.

/// An error encountered while parsing an XMP packet.
///
/// Serialization is infallible (see [`crate::XmpMeta::to_packet`]); these errors arise only on the
/// read path. Variants carry dynamic context — a short detail and, where the parser has it, the
/// byte `offset` into the packet — so a malformed packet is easy to diagnose.
///
/// `quick-xml` is an internal implementation detail: its error type is **not** re-exposed here (XML
/// lexing failures are captured as [`XmpError::Xml`] with an owned message), so the XML backend can
/// change without breaking this crate's public API.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum XmpError {
    /// The packet is not valid UTF-8, or declares an unsupported text encoding. gamut-xmp reads and
    /// writes UTF-8; Part 1 §7.1 also permits UTF-16/32, which are not implemented.
    #[error("XMP encoding: {0}")]
    Encoding(&'static str),

    /// The RDF/XML could not be lexed. `detail` is the underlying lexer's message; `offset` is the
    /// byte position in the packet.
    #[error("XMP: malformed XML at byte {offset}: {detail}")]
    Xml {
        /// A human-readable description of the lexing failure.
        detail: String,
        /// Byte offset into the packet where the failure was detected.
        offset: u64,
    },

    /// No `rdf:RDF` element was found in the packet (Part 1 §7.4).
    #[error("XMP: no rdf:RDF root element found")]
    MissingRdf,

    /// A namespace prefix was used without an in-scope `xmlns` declaration.
    #[error("XMP: undeclared namespace prefix '{prefix}' at byte {offset}")]
    UnknownPrefix {
        /// The unresolved prefix.
        prefix: String,
        /// Byte offset where the prefix was used.
        offset: u64,
    },

    /// An RDF/XML form that XMP does not permit (Part 1 §7.5/§7.9): e.g. `rdf:parseType="Literal"`
    /// or `"Collection"`, or a top-level typed node.
    #[error("XMP: unsupported RDF/XML form at byte {offset}: {construct}")]
    UnsupportedForm {
        /// The offending construct.
        construct: &'static str,
        /// Byte offset where it appears.
        offset: u64,
    },

    /// A construct the spec explicitly prohibits (Part 1 §7.8/§7.9.3): e.g. `rdf:_n` array items,
    /// or an `rdf:value` that carries `xml:lang` or nested general qualifiers.
    #[error("XMP: prohibited construct at byte {offset}: {construct}")]
    Prohibited {
        /// The offending construct.
        construct: &'static str,
        /// Byte offset where it appears.
        offset: u64,
    },

    /// A language alternative (`rdf:Alt`) held two items with the same `xml:lang`; Part 1 §8.2.2.4
    /// requires the language tags to be unique.
    #[error("XMP: duplicate xml:lang '{0}' in an alternative array")]
    DuplicateLang(String),
}

/// A specialized [`Result`](core::result::Result) for the XMP read path.
pub type Result<T> = core::result::Result<T, XmpError>;

impl From<XmpError> for gamut_core::Error {
    /// Funnels an [`XmpError`] into the workspace's unified [`gamut_core::Error`] so the
    /// `gamut-metadata` facade and the format crates can present one error surface. gamut-core's
    /// error carries only static messages, so the dynamic detail is dropped; reach for [`XmpError`]
    /// directly when that detail matters.
    fn from(err: XmpError) -> Self {
        match err {
            XmpError::Encoding(_) | XmpError::UnsupportedForm { .. } => {
                gamut_core::Error::Unsupported("XMP: unsupported feature")
            }
            _ => gamut_core::Error::InvalidInput("XMP: invalid metadata packet"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_dynamic_context() {
        let e = XmpError::Xml {
            detail: "unexpected end of input".into(),
            offset: 42,
        };
        let s = e.to_string();
        assert!(s.contains("byte 42"), "offset must surface: {s}");
        assert!(s.contains("unexpected end of input"), "detail must surface: {s}");
    }

    #[test]
    fn maps_to_core_unsupported_vs_invalid() {
        // Encoding / unsupported-form become Unsupported; everything else is invalid input.
        assert!(matches!(
            gamut_core::Error::from(XmpError::Encoding("x")),
            gamut_core::Error::Unsupported(_)
        ));
        assert!(matches!(
            gamut_core::Error::from(XmpError::UnsupportedForm {
                construct: "parseType=Literal",
                offset: 0
            }),
            gamut_core::Error::Unsupported(_)
        ));
        assert!(matches!(
            gamut_core::Error::from(XmpError::MissingRdf),
            gamut_core::Error::InvalidInput(_)
        ));
        assert!(matches!(
            gamut_core::Error::from(XmpError::DuplicateLang("en".into())),
            gamut_core::Error::InvalidInput(_)
        ));
    }
}
