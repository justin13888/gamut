//! The typed ISOBMFF/HEIF still-image box tree.
//!
//! These structs model the *structure* of a single-image ISOBMFF file — its `ftyp` brands and the
//! `meta` image items with their properties and payloads — and never the coded bitstream itself,
//! which stays opaque (carried as [`PropertyKind::CodecConfiguration`] and [`Item::payload`]). This
//! is the codec-agnostic layer both AVIF (`av01`/`av1C`) and HEIC (`hvc1`/`hvcC`) build on.
//!
//! [`crate::write`] serialises an [`IsoBmffImage`]; [`crate::read`] parses one back. The model is
//! normalised so the two are inverse for files this crate writes: it stores each item's resolved
//! [`payload`](Item::payload) (not raw `iloc` offsets) and its per-item [`properties`](Item::properties)
//! list (not raw `ipco` indices), so `read(&write(&img)) == img`.

/// A parsed or constructed ISOBMFF still-image file: its `ftyp` brands, the id of the primary
/// (displayed) item, and the image items.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsoBmffImage {
    /// The `ftyp` major brand (e.g. `*b"avif"`).
    pub major_brand: [u8; 4],
    /// The `ftyp` minor version (typically `0`).
    pub minor_version: u32,
    /// The `ftyp` compatible brands, in file order (e.g. `avif`/`mif1`/`miaf`/`MA1A`).
    pub compatible_brands: Vec<[u8; 4]>,
    /// The `pitm` primary item id — the image a reader displays.
    pub primary_item_id: u16,
    /// The image items, in file order. The first/primary item is the coded image; further items
    /// are auxiliaries (e.g. a future alpha plane).
    pub items: Vec<Item>,
}

/// One image item: its id, four-character type, optional name, the properties associated with it
/// (in association order), and its payload bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The item id, unique within the file and referenced by `pitm`/`iloc`/`iinf`/`ipma`.
    pub id: u16,
    /// The item type four-character code (e.g. `*b"av01"` for an AV1 image, `*b"hvc1"` for HEVC).
    pub item_type: [u8; 4],
    /// The item name (`infe` `item_name`), usually empty. Must be valid UTF-8 with no interior NUL.
    pub name: String,
    /// The item's properties, in `ipma` association order. The codec configuration is conventionally
    /// first and `essential`.
    pub properties: Vec<Property>,
    /// The item's payload — for the primary image, the coded bitstream placed in `mdat` (e.g. the AV1
    /// temporal unit). Opaque to this crate.
    pub payload: Vec<u8>,
}

/// An item property together with whether a reader must understand it to render the item
/// (`essential`, MIAF §7.3.6 / ISO/IEC 23008-12 §9.3.1). Transformative properties and the codec
/// configuration are essential; descriptive ones (`ispe`/`pixi`/`colr`) are not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    /// Whether the property is marked essential in `ipma` (the high bit of the association entry).
    pub essential: bool,
    /// The property itself.
    pub kind: PropertyKind,
}

/// An item property box (`ipco` child). Recognised HEIF properties are modelled structurally; any
/// other property box (including a codec configuration) is carried verbatim so it round-trips.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyKind {
    /// `ispe` image spatial extents — the stored image dimensions (ISO/IEC 23008-12 §6.5.3).
    ImageSpatialExtents {
        /// Image width in pixels.
        width: u32,
        /// Image height in pixels.
        height: u32,
    },
    /// `pixi` pixel information — the bit depth of each channel, in order (ISO/IEC 23008-12 §6.5.6).
    /// The length is the channel count (3 for colour, 1 for monochrome).
    PixelInformation {
        /// Bits per channel, one entry per channel.
        bits_per_channel: Vec<u8>,
    },
    /// `colr` colour information (ISOBMFF `ColourInformationBox`).
    Colour(ColourInformation),
    /// `irot` image rotation — anti-clockwise quarter turns, `0..=3` (ISO/IEC 23008-12 §6.5.10).
    Rotation(u8),
    /// `imir` image mirror — axis `0` (vertical, left↔right) or `1` (horizontal, top↔bottom)
    /// (ISO/IEC 23008-12 §6.5.12).
    Mirror(u8),
    /// A codec configuration property (e.g. `av1C`, `hvcC`) carried as opaque bytes — the container
    /// never interprets the coded-format record. `kind` is the box type; `data` is its body.
    CodecConfiguration {
        /// The property box type (e.g. `*b"av1C"`).
        kind: [u8; 4],
        /// The property box body, verbatim.
        data: Vec<u8>,
    },
    /// Any other (unrecognised) property box, preserved verbatim for round-tripping. `kind` is the
    /// box type; `data` is its body.
    Other {
        /// The property box type.
        kind: [u8; 4],
        /// The property box body, verbatim.
        data: Vec<u8>,
    },
}

/// The contents of a `colr` box. Only the `nclx` (CICP code points) form is modelled; an ICC
/// profile (`rICC`/`prof`) round-trips as [`PropertyKind::Other`] until a consumer needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColourInformation {
    /// `nclx` on-screen colour: CICP code points plus the full-range flag.
    Nclx(NclxColr),
}

/// The `nclx` colour information written into a `colr` box (CICP code points, ITU-T H.273). For an
/// AV1 image `matrix_coefficients` and `full_range` must match the sequence header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NclxColr {
    /// CICP colour primaries.
    pub colour_primaries: u16,
    /// CICP transfer characteristics.
    pub transfer_characteristics: u16,
    /// CICP matrix coefficients.
    pub matrix_coefficients: u16,
    /// Full-range (vs limited-range) flag.
    pub full_range: bool,
}
