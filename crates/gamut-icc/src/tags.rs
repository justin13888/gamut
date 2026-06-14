//! The tag table: the signatures and byte offsets that index a profile's tag element data.

use std::collections::HashSet;

use gamut_core::{Error, Result};

use crate::bytes::ByteReader;
use crate::primitives::Signature;

/// One row of the on-disk tag table (ICC.1:2022 §7.3): a tag signature plus the byte offset and
/// size of its element data within the profile.
///
/// This is an encoding detail — the parsed [`crate::IccProfile`] stores decoded tags, not offsets,
/// and the serializer recomputes the table — so it is internal to the crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TagEntry {
    pub(crate) signature: Signature,
    pub(crate) offset: u32,
    pub(crate) size: u32,
}

/// Parses the tag table: a `u32` count at offset 128 followed by `count` twelve-byte rows
/// (ICC.1:2022 §7.3).
///
/// Validates that the table itself fits within `profile` and that no tag signature is duplicated.
/// Each tag's element-data bounds are checked by the caller as it is decoded.
pub(crate) fn parse_tag_table(profile: &[u8]) -> Result<Vec<TagEntry>> {
    let mut r = ByteReader::at(profile, 128)?;
    let count = r.u32()? as usize;
    // Reject a count that cannot fit before allocating; `12 * count + 132` must be within bounds.
    let table_end = count
        .checked_mul(12)
        .and_then(|n| n.checked_add(132))
        .ok_or(Error::InvalidInput("icc: tag count overflow"))?;
    if table_end > profile.len() {
        return Err(Error::InvalidInput("icc: tag table exceeds profile"));
    }
    let mut entries = Vec::with_capacity(count);
    let mut seen = HashSet::with_capacity(count);
    for _ in 0..count {
        let signature = r.signature()?;
        if !seen.insert(signature.0) {
            return Err(Error::InvalidInput("icc: duplicate tag signature"));
        }
        let offset = r.u32()?;
        let size = r.u32()?;
        entries.push(TagEntry {
            signature,
            offset,
            size,
        });
    }
    Ok(entries)
}

/// Well-known tag signatures a baseline profile carries (ICC.1:2022 §9).
///
/// A convenience catalogue of the common tags; the parser accepts *any* signature, so this is not
/// exhaustive and is not on the parse path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KnownTag {
    /// `desc` — the profile description (`textDescriptionType` v2 / `multiLocalizedUnicodeType` v4).
    ProfileDescription,
    /// `cprt` — the copyright string.
    Copyright,
    /// `wtpt` — the media white point (`XYZType`).
    MediaWhitePoint,
    /// `rXYZ` — the red colorant column (`XYZType`).
    RedColorant,
    /// `gXYZ` — the green colorant column (`XYZType`).
    GreenColorant,
    /// `bXYZ` — the blue colorant column (`XYZType`).
    BlueColorant,
    /// `rTRC` — the red tone-response curve (`curveType` / `parametricCurveType`).
    RedTrc,
    /// `gTRC` — the green tone-response curve.
    GreenTrc,
    /// `bTRC` — the blue tone-response curve.
    BlueTrc,
    /// `A2B0` — the device-to-PCS lookup transform for the perceptual intent.
    AToB0,
    /// `B2A0` — the PCS-to-device lookup transform for the perceptual intent.
    BToA0,
    /// `chad` — the chromatic-adaptation matrix (`s15Fixed16ArrayType`).
    ChromaticAdaptation,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 128-byte zero header is the minimum any profile bytes start with; tag-table tests append
    /// the count and rows after it.
    fn header_padding() -> Vec<u8> {
        vec![0u8; 128]
    }

    #[test]
    fn parses_rows_after_the_count() {
        let mut b = header_padding();
        b.extend_from_slice(&2u32.to_be_bytes()); // count
        for (sig, off, size) in [(b"wtpt", 200u32, 20u32), (b"rXYZ", 220, 20)] {
            b.extend_from_slice(sig);
            b.extend_from_slice(&off.to_be_bytes());
            b.extend_from_slice(&size.to_be_bytes());
        }
        b.resize(240, 0); // room for the referenced data
        let entries = parse_tag_table(&b).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].signature, Signature(*b"wtpt"));
        assert_eq!(entries[1].offset, 220);
    }

    #[test]
    fn rejects_duplicate_signature() {
        let mut b = header_padding();
        b.extend_from_slice(&2u32.to_be_bytes());
        for _ in 0..2 {
            b.extend_from_slice(b"wtpt");
            b.extend_from_slice(&156u32.to_be_bytes());
            b.extend_from_slice(&0u32.to_be_bytes());
        }
        assert!(parse_tag_table(&b).is_err());
    }

    #[test]
    fn rejects_count_that_overflows_the_buffer() {
        let mut b = header_padding();
        b.extend_from_slice(&0xFFFF_FFFFu32.to_be_bytes()); // absurd count
        assert!(parse_tag_table(&b).is_err());
    }

    #[test]
    fn rejects_truncated_table() {
        let mut b = header_padding();
        b.extend_from_slice(&1u32.to_be_bytes()); // count 1, but no row follows
        assert!(parse_tag_table(&b).is_err());
    }
}
