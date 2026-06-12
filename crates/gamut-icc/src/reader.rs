//! The ICC profile reader.

/// Reader for an ICC profile blob.
///
/// Will parse the 128-byte [`crate::ProfileHeader`], the tag table, and each tag's element data
/// (dispatched on its [`crate::TagType`]) into an [`crate::IccProfile`]. Implementation pending
/// (see issue #34).
pub struct IccReader;
