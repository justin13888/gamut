//! The ICC profile writer.

/// Writer for an ICC profile blob.
///
/// Will serialise an [`crate::IccProfile`] back to bytes: the header, the tag table, and the
/// (offset-aligned) tag element data, recomputing `size` and the profile ID. A round-trip
/// (parse → serialize) must preserve the profile's color behaviour. Implementation pending (see
/// issue #34).
pub struct IccWriter;
