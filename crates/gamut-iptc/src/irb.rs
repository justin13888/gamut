//! The Photoshop Image Resource Block carrier for legacy IIM.

/// A parsed Photoshop Image Resource (the `8BIM` block stream, typically in a JPEG `APP13`
/// segment). Legacy IPTC-IIM is carried in the resource with id `0x0404`.
pub struct PhotoshopIrb {
    /// The image-resource blocks, in file order.
    pub blocks: Vec<IrbBlock>,
}

/// One Photoshop image-resource block.
pub struct IrbBlock {
    /// The 2-byte resource id (`0x0404` is the IPTC-IIM block).
    pub resource_id: u16,
    /// The optional Pascal-string resource name (usually empty).
    pub name: String,
    /// The raw resource data (for `0x0404`, the IIM dataset stream).
    pub data: Vec<u8>,
}
