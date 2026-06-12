//! The tag table: the signatures and offsets that index a profile's tag element data.

/// A four-byte tag signature (e.g. `rXYZ`, `desc`), as stored in the tag table.
pub struct TagSignature(pub [u8; 4]);

/// One row of the profile's tag table (ICC.1:2022 §7.3): a tag signature and the offset/size of
/// its element data within the profile.
pub struct TagEntry {
    /// The tag's four-byte signature.
    pub signature: TagSignature,
    /// Byte offset of the tag's element data from the start of the profile.
    pub offset: u32,
    /// Size in bytes of the tag's element data.
    pub size: u32,
}

/// The well-known tag signatures a baseline profile carries (ICC.1:2022 §9). Representative subset;
/// the full registry is filled in during implementation.
pub enum KnownTag {
    /// `desc` — the profile description (`textDescriptionType` v2 / `multiLocalizedUnicodeType` v4).
    ProfileDescription,
    /// `cprt` — the copyright string.
    Copyright,
    /// `wtpt` — the media white point (`XYZType`).
    MediaWhitePoint,
    /// `rXYZ` / `gXYZ` / `bXYZ` — the RGB colorant matrix columns (`XYZType`).
    RedColorant,
    /// `gXYZ` — green colorant.
    GreenColorant,
    /// `bXYZ` — blue colorant.
    BlueColorant,
    /// `rTRC` / `gTRC` / `bTRC` — the per-channel tone-response curves (`curveType` /
    /// `parametricCurveType`).
    RedTrc,
    /// `gTRC` — green tone-response curve.
    GreenTrc,
    /// `bTRC` — blue tone-response curve.
    BlueTrc,
    /// `A2B0` — the device-to-PCS lookup transform for the perceptual intent (`lut*` / `lutAToB`).
    AToB0,
    /// `B2A0` — the PCS-to-device lookup transform for the perceptual intent (`lut*` / `lutBToA`).
    BToA0,
    /// `chad` — the chromatic-adaptation matrix (`s15Fixed16ArrayType`).
    ChromaticAdaptation,
}
