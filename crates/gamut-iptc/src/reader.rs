//! The IPTC reader.

/// Reader for IPTC metadata.
///
/// Will parse the Photoshop IRB (`8BIM`) stream to locate the `0x0404` resource and decode its IIM
/// datasets, and parse IPTC Core/Extension from an XMP packet via [`gamut_xmp`]. Implementation
/// pending (see issue #34).
pub struct IptcReader;
