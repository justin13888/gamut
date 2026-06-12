//! Extracting a unified model from metadata blocks.

/// Parses a set of [`crate::MetadataBlock`]s into a unified [`crate::Metadata`].
///
/// Will dispatch each block to the matching per-format parser (EXIF → `gamut-exif`, XMP →
/// `gamut-xmp`, ICC → `gamut-icc`, IPTC → `gamut-iptc`), then reconcile overlapping data into one
/// coherent view. Implementation pending (see issue #34).
pub struct MetadataExtractor;
