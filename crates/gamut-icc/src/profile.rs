//! The parsed profile: the header plus its tag table.

use crate::header::ProfileHeader;
use crate::tags::TagEntry;

/// A parsed ICC profile.
///
/// Holds the decoded [`ProfileHeader`] and the tag table. The parsed element data for each tag
/// (curves, matrices, CLUTs, text) is materialised through the tag table in the implementation
/// phase; this scaffold models the structural skeleton only.
pub struct IccProfile {
    /// The 128-byte profile header.
    pub header: ProfileHeader,
    /// The tag table — one entry per tag, in tag order.
    pub tag_table: Vec<TagEntry>,
}
