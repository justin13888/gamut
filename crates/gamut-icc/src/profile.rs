//! The parsed ICC profile: its header and decoded tags.

use crate::error::{IccError, Result};
use crate::header::ProfileHeader;
use crate::primitives::Signature;
use crate::tag_types::{TagData, decode_tag};
use crate::tags::{parse_tag_table, tag_table_end};

/// A parsed ICC profile: the [`ProfileHeader`] plus its tags, decoded and in file order.
///
/// Parse with [`IccProfile::parse`]; look up a tag with [`IccProfile::get`]. The on-disk tag table
/// (byte offsets and sizes) is an encoding detail reconstructed by the serializer, so it is not part
/// of this model — callers manipulate decoded data, not offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IccProfile {
    /// The 128-byte profile header.
    pub header: ProfileHeader,
    /// The profile's tags in file order, each a `(signature, decoded data)` pair.
    pub tags: Vec<(Signature, TagData)>,
}

impl IccProfile {
    /// Parses a complete ICC profile from its bytes (lenient; see [`crate::IccReader`] for strict
    /// parsing).
    ///
    /// # Errors
    ///
    /// Returns [`IccError::Malformed`] if the header, the tag table, or any tag's element data is
    /// malformed or points outside the profile.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        Self::parse_with(bytes, false)
    }

    /// Serializes the profile to a fresh, spec-valid byte vector, preserving the header's stored
    /// profile ID. Use [`crate::IccWriter`] to recompute the ID instead.
    ///
    /// # Errors
    ///
    /// Returns [`IccError::Malformed`] if the model violates an invariant serialization relies on:
    /// a duplicate tag signature, LUT tables or curve sets whose lengths contradict their declared
    /// channel counts, an 8-bit CLUT sample over 255, an over-long fixed-width name field, or
    /// non-ASCII text in an ASCII element. These are only reachable with hand-built data — the
    /// decoder establishes every such invariant, so a profile produced by [`IccProfile::parse`]
    /// always serializes.
    pub fn to_bytes(&self) -> Result<Vec<u8>> {
        crate::writer::write_profile(self, false)
    }

    /// The parse implementation shared by [`IccProfile::parse`] and [`crate::IccReader`]; `strict`
    /// adds conformance checks the lenient default skips.
    pub(crate) fn parse_with(bytes: &[u8], strict: bool) -> Result<Self> {
        let header = ProfileHeader::parse(bytes)?;
        if strict && header.reserved.iter().any(|&b| b != 0) {
            return Err(IccError::Malformed(
                "icc: nonzero reserved header bytes (strict)",
            ));
        }
        let entries = parse_tag_table(bytes)?;
        let data_start = tag_table_end(entries.len());
        let mut tags = Vec::with_capacity(entries.len());
        for entry in entries {
            let start = entry.offset as usize;
            if strict && start < data_start {
                return Err(IccError::Malformed(
                    "icc: tag data overlaps the header or tag table (strict)",
                ));
            }
            let end = start
                .checked_add(entry.size as usize)
                .ok_or(IccError::Malformed("icc: tag size overflow"))?;
            let element = bytes
                .get(start..end)
                .ok_or(IccError::Malformed("icc: tag data out of bounds"))?;
            tags.push((entry.signature, decode_tag(element)?));
        }
        Ok(Self { header, tags })
    }

    /// The decoded data of the tag with the given signature, if present.
    ///
    /// Accepts anything convertible to a [`Signature`] — a [`crate::KnownTag`], four bytes
    /// (`profile.get(*b"wtpt")`), or a `Signature` itself.
    #[must_use]
    pub fn get(&self, signature: impl Into<Signature>) -> Option<&TagData> {
        let signature = signature.into();
        self.tags
            .iter()
            .find(|(s, _)| *s == signature)
            .map(|(_, data)| data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal valid 128-byte header (the required closed-registry fields plus `acsp`); the rest
    /// is zero, which is a valid "unspecified" for every open field.
    fn header() -> Vec<u8> {
        let mut b = vec![0u8; 128];
        b[12..16].copy_from_slice(b"mntr"); // device class
        b[16..20].copy_from_slice(b"RGB "); // data colour space
        b[20..24].copy_from_slice(b"XYZ "); // PCS
        b[36..40].copy_from_slice(b"acsp"); // magic
        b // rendering intent at 64 is 0 (perceptual)
    }

    #[test]
    fn model_stays_eq() {
        // Compile-time guard: every TagData payload is integer-backed, so the whole model is `Eq`.
        // A future variant carrying an `f64` would silently strip `Eq` from `TagData`, `IccProfile`
        // and everything between; fail here instead so the loss is a deliberate decision.
        fn assert_eq_capable<T: Eq>() {}
        assert_eq_capable::<TagData>();
        assert_eq_capable::<IccProfile>();
    }

    #[test]
    fn parses_minimal_profile_and_looks_up_tags() {
        let mut b = header();
        b.extend_from_slice(&1u32.to_be_bytes()); // tag count
        let offset = 128 + 4 + 12; // first byte after the one-row table
        b.extend_from_slice(b"wtpt");
        b.extend_from_slice(&(offset as u32).to_be_bytes());
        b.extend_from_slice(&12u32.to_be_bytes());
        b.extend_from_slice(b"zzzz\x00\x00\x00\x00\x00\x00\x00\x00"); // unknown 12-byte element

        let profile = IccProfile::parse(&b).unwrap();
        assert_eq!(profile.tags.len(), 1);
        // An unmodelled element type is preserved as Raw and is still locatable by signature.
        assert!(matches!(
            profile.get(Signature(*b"wtpt")),
            Some(TagData::Raw { .. })
        ));
        assert!(profile.get(Signature(*b"abcd")).is_none());
    }

    #[test]
    fn rejects_tag_data_out_of_bounds() {
        let mut b = header();
        b.extend_from_slice(&1u32.to_be_bytes());
        b.extend_from_slice(b"wtpt");
        b.extend_from_slice(&10_000u32.to_be_bytes()); // offset past EOF
        b.extend_from_slice(&20u32.to_be_bytes());
        assert!(IccProfile::parse(&b).is_err());
    }

    #[test]
    fn empty_tag_table_is_valid() {
        let mut b = header();
        b.extend_from_slice(&0u32.to_be_bytes()); // zero tags
        let profile = IccProfile::parse(&b).unwrap();
        assert!(profile.tags.is_empty());
    }

    #[test]
    fn strict_mode_rejects_nonzero_reserved_header_bytes() {
        let mut b = header();
        b.extend_from_slice(&0u32.to_be_bytes()); // empty tag table
        b[100] = 1; // a reserved header byte (100..128)
        assert!(IccProfile::parse(&b).is_ok()); // lenient tolerates it
        assert!(IccProfile::parse_with(&b, true).is_err()); // strict rejects it
    }

    #[test]
    fn strict_mode_rejects_tag_overlapping_the_table() {
        let mut b = header();
        b.extend_from_slice(&1u32.to_be_bytes()); // one tag
        b.extend_from_slice(b"wtpt");
        b.extend_from_slice(&8u32.to_be_bytes()); // offset 8 — inside the header
        b.extend_from_slice(&12u32.to_be_bytes());
        assert!(IccProfile::parse(&b).is_ok()); // lenient allows odd offsets
        assert!(IccProfile::parse_with(&b, true).is_err()); // strict requires offsets past the table
    }
}
