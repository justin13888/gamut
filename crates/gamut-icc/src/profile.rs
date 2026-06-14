//! The parsed ICC profile: its header and decoded tags.

use gamut_core::{Error, Result};
use md5::{Digest, Md5};

use crate::header::{ProfileHeader, ProfileId};
use crate::primitives::Signature;
use crate::tag_types::{TagData, decode_tag};
use crate::tags::parse_tag_table;

/// A parsed ICC profile: the [`ProfileHeader`] plus its tags, decoded and in file order.
///
/// Parse with [`IccProfile::parse`]; look up a tag with [`IccProfile::get`]. The on-disk tag table
/// (byte offsets and sizes) is an encoding detail reconstructed by the serializer, so it is not part
/// of this model — callers manipulate decoded data, not offsets.
#[derive(Debug, Clone, PartialEq)]
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
    /// Returns [`Error::InvalidInput`] if the header, the tag table, or any tag's element data is
    /// malformed or points outside the profile.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        Self::parse_with(bytes, false)
    }

    /// Serializes the profile to a fresh, spec-valid byte vector, preserving the header's stored
    /// profile ID. Use [`crate::IccWriter`] to recompute the ID instead.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        crate::writer::write_profile(self, false)
    }

    /// The parse implementation shared by [`IccProfile::parse`] and [`crate::IccReader`]; `strict`
    /// adds conformance checks the lenient default skips.
    pub(crate) fn parse_with(bytes: &[u8], strict: bool) -> Result<Self> {
        let header = ProfileHeader::parse(bytes)?;
        if strict && header.reserved.iter().any(|&b| b != 0) {
            return Err(Error::InvalidInput(
                "icc: nonzero reserved header bytes (strict)",
            ));
        }
        let entries = parse_tag_table(bytes)?;
        let data_start = 128 + 4 + 12 * entries.len();
        let mut tags = Vec::with_capacity(entries.len());
        for entry in entries {
            let start = entry.offset as usize;
            if strict && start < data_start {
                return Err(Error::InvalidInput(
                    "icc: tag data overlaps the header or tag table (strict)",
                ));
            }
            let end = start
                .checked_add(entry.size as usize)
                .ok_or(Error::InvalidInput("icc: tag size overflow"))?;
            let element = bytes
                .get(start..end)
                .ok_or(Error::InvalidInput("icc: tag data out of bounds"))?;
            tags.push((entry.signature, decode_tag(element)?));
        }
        Ok(Self { header, tags })
    }

    /// The decoded data of the tag with the given `signature`, if present.
    #[must_use]
    pub fn get(&self, signature: Signature) -> Option<&TagData> {
        self.tags
            .iter()
            .find(|(s, _)| *s == signature)
            .map(|(_, data)| data)
    }

    /// Computes the profile ID (ICC.1:2022 §7.2.18): the MD5 of a fully serialized profile with the
    /// profile-flags (bytes 44–47), rendering-intent (64–67) and profile-ID (84–99) fields zeroed
    /// first, as the spec requires.
    #[must_use]
    pub fn compute_profile_id(profile_bytes: &[u8]) -> ProfileId {
        let mut buf = profile_bytes.to_vec();
        for range in [44..48usize, 64..68, 84..100] {
            if let Some(field) = buf.get_mut(range) {
                field.fill(0);
            }
        }
        ProfileId(Md5::digest(&buf).into())
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
    fn profile_id_excludes_the_zeroed_fields() {
        let mut base = header();
        base.extend_from_slice(&0u32.to_be_bytes()); // empty tag table → 132 bytes
        let id = IccProfile::compute_profile_id(&base);

        // The flags (44), rendering-intent (64) and profile-ID (84–99) regions are zeroed first, so
        // changing a byte in any of them leaves the ID unchanged.
        for offset in [44usize, 64, 90] {
            let mut poked = base.clone();
            poked[offset] = 0xFF;
            assert_eq!(
                IccProfile::compute_profile_id(&poked),
                id,
                "offset {offset} should be excluded from the ID"
            );
        }
        // A byte outside those regions does change the ID.
        let mut other = base.clone();
        other[40] = 0xFF; // primary platform
        assert_ne!(IccProfile::compute_profile_id(&other), id);
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
