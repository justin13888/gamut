//! The IPTC writer.

/// Writer for IPTC metadata.
///
/// Will serialise IIM datasets back into a Photoshop IRB (`8BIM`) `0x0404` resource, and IPTC
/// Core/Extension into an XMP packet via [`gamut_xmp`] — keeping the two carriers consistent per
/// the reconciliation rules. Implementation pending (see issue #34).
pub struct IptcWriter;
