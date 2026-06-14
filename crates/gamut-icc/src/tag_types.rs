//! The decoded element data a tag can hold (ICC.1:2022 §10).

use gamut_core::Result;

use crate::primitives::Signature;

/// The decoded data of a tag element.
///
/// Each variant models one ICC element type. [`TagData::Raw`] carries any element type gamut-icc
/// does not decode semantically — verbatim — so every tag round-trips byte-for-byte regardless of
/// whether it is modelled. The enum is `#[non_exhaustive]`: variants are added as more element
/// types gain semantic decoders.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum TagData {
    /// An element gamut-icc does not model semantically: the complete element bytes verbatim,
    /// including the leading four-byte type signature and its four reserved bytes. Re-emitted
    /// exactly on serialization.
    Raw {
        /// The element's four-byte type signature (the first four bytes of `bytes`).
        type_sig: Signature,
        /// The complete element bytes.
        bytes: Vec<u8>,
    },
}

/// Decodes one tag element from its bytes; the element begins with its four-byte type signature
/// followed by four reserved bytes (ICC.1:2022 §10).
///
/// Every element is currently preserved as [`TagData::Raw`]; semantic decoders for the modelled
/// element types are layered on in later phases, falling back to `Raw` for the rest.
pub(crate) fn decode_tag(element: &[u8]) -> Result<TagData> {
    let type_sig = element_type_signature(element);
    Ok(TagData::Raw {
        type_sig,
        bytes: element.to_vec(),
    })
}

/// The element's four-byte type signature, or [`Signature::ZERO`] for an element shorter than four
/// bytes (a malformed element the caller still round-trips verbatim).
fn element_type_signature(element: &[u8]) -> Signature {
    match element.get(..4) {
        Some(s) => Signature([s[0], s[1], s[2], s[3]]),
        None => Signature::ZERO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_element_is_preserved_as_raw() {
        let element = b"zzzz\x00\x00\x00\x00payload".to_vec();
        let TagData::Raw { type_sig, bytes } = decode_tag(&element).unwrap();
        assert_eq!(type_sig, Signature(*b"zzzz"));
        assert_eq!(bytes, element); // byte-for-byte verbatim
    }

    #[test]
    fn short_element_has_zero_type_signature() {
        let TagData::Raw { type_sig, bytes } = decode_tag(&[1, 2]).unwrap();
        assert_eq!(type_sig, Signature::ZERO);
        assert_eq!(bytes, vec![1, 2]);
    }
}
