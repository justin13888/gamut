//! Serializing a unified model back to metadata blocks.

/// Serializes a [`crate::Metadata`] back into per-format byte blocks for a container to embed.
///
/// Will produce the EXIF blob, XMP packet, ICC profile, and IPTC payload(s) — the inverse of
/// [`crate::MetadataExtractor`] — so a container crate can write them as the appropriate chunk/item.
/// A round-trip (extract → embed) must preserve the metadata losslessly. Implementation pending
/// (see issue #34).
pub struct MetadataEmbedder;
