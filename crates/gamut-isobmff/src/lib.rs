//! ISO Base Media File Format (ISOBMFF) container for still images — the structural layer the AVIF
//! and HEIC codecs share.
//!
//! This crate owns the *container*, not the codec: it models the box tree of a single-image ISOBMFF
//! file (`ftyp` + a `meta` box of image items + `mdat`) and leaves the coded bitstream opaque
//! ([`PropertyKind::CodecConfiguration`] for the `av1C`/`hvcC` record, [`Item::payload`] for the
//! samples). [`write`] serialises an [`IsoBmffImage`]; [`read`] parses one back. The two are inverse
//! for any file this crate writes (`read(&write(&img)) == img`).
//!
//! It is image-first: the supported boxes are the HEIF still-image set (`ftyp`, `meta` with
//! `hdlr`/`pitm`/`iloc` v0/`iinf`+`infe` v2/`iprp`, and the `ispe`/`pixi`/`colr`/`irot`/`imir`
//! properties), plus opaque codec-configuration and unrecognised properties carried verbatim. Image
//! sequences/tracks, `iloc` v1/v2, multi-extent items, and `idat`/`grid`/alpha are out of scope —
//! see this crate's `STATUS.md`. Box byte layouts follow ISO/IEC 14496-12 (ISOBMFF) and ISO/IEC
//! 23008-12 (HEIF); see `references/isobmff`.
//!
//! ```
//! use gamut_isobmff::{IsoBmffImage, Item, Property, PropertyKind, read, write};
//!
//! let img = IsoBmffImage {
//!     major_brand: *b"avif",
//!     minor_version: 0,
//!     compatible_brands: vec![*b"avif", *b"mif1", *b"miaf"],
//!     primary_item_id: 1,
//!     items: vec![Item {
//!         id: 1,
//!         item_type: *b"av01",
//!         name: String::new(),
//!         properties: vec![Property {
//!             essential: false,
//!             kind: PropertyKind::ImageSpatialExtents { width: 64, height: 64 },
//!         }],
//!         payload: vec![1, 2, 3, 4], // the coded bitstream (opaque to this crate)
//!     }],
//! };
//! let bytes = write(&img);
//! assert_eq!(read(&bytes).unwrap(), img);
//! ```
#![forbid(unsafe_code)]

mod boxes;
mod model;
mod reader;
mod writer;

pub use model::{ColourInformation, IsoBmffImage, Item, NclxColr, Property, PropertyKind};
pub use reader::read;
pub use writer::write;
