//! HEIC/HEIF still-image **container decoder** — HEVC intra image items in an ISOBMFF container.
//!
//! This crate is decode-side and, in this slice (issue #238), covers the **container layer** only:
//! it parses a HEIF/HEIC file into a byte-exact representation and a role-typed semantic view. It
//! does **not** encode HEIF (gamut is decode-only for this format, see `references/heif`), and it
//! does not yet touch the coded bitstream — `hvcC` record parsing, NAL demux, and HEVC-intra pixel
//! decoding are later slices.
//!
//! # Two layers
//!
//! - [`HeifContainer`] — the **total, byte-accounting** representation. Its
//!   [`segments`](HeifContainer::segments) are contiguous, non-overlapping, and cover every byte of
//!   the input, so it is *structurally impossible to ignore any bits*: unknown top-level boxes are
//!   surfaced verbatim ([`SegmentKind::Box`]), an appended vendor stream (a second top-level `ftyp`,
//!   e.g. a Samsung motion-photo MP4) is retained opaquely ([`SegmentKind::AppendedStream`]), and
//!   trailing non-box bytes become an explicit [`SegmentKind::Trailer`]. Boxes inside `meta` that
//!   the semantic parse does not consume are surfaced as [`UnknownBox`]es. Vendor motion-photo
//!   *semantics* stay downstream; this layer exposes the container's true representation.
//! - [`HeifImage`] — the **role-typed semantic view** over the primary still-image stream, wrapping
//!   [`gamut_isobmff::IsoBmffImage`]. It reads roles (primary image, alpha/depth auxiliaries,
//!   thumbnails, Exif/XMP metadata, grid/overlay derivations) as computed lenses over the items,
//!   never duplicating state.
//!
//! The box tree itself is the shared [`gamut_isobmff`] primitive (`ftyp`/`meta`/`iloc`/`iinf`/…);
//! this crate layers the HEIF still-image profile and the byte-accounting guarantee on top.
//!
//! # Deferred to later slices
//!
//! Typed `hvcC` HEVCDecoderConfigurationRecord parsing and NAL-unit demux/classification; the
//! [`gamut_core::DecodeImage`] implementation and the HEVC-intra pixel pipeline; and the libheif
//! differential oracle. Image *sequences* (`msf1`/`hevc`/`hevx` tracks) are permanently out of
//! scope (gamut is image-first). See this crate's `STATUS.md`.
//!
//! # Example
//!
//! ```
//! use gamut_heic::HeifContainer;
//! use gamut_isobmff::{IsoBmffImage, Item, Property, PropertyKind, write};
//!
//! // Build a minimal HEVC still with gamut-isobmff, then parse it back.
//! let img = IsoBmffImage {
//!     major_brand: *b"heic",
//!     minor_version: 0,
//!     compatible_brands: vec![*b"heic", *b"mif1"],
//!     primary_item_id: 1,
//!     items: vec![Item {
//!         id: 1,
//!         item_type: *b"hvc1",
//!         name: String::new(),
//!         content_type: None,
//!         content_encoding: None,
//!         hidden: false,
//!         references: vec![],
//!         properties: vec![Property {
//!             essential: true,
//!             kind: PropertyKind::CodecConfiguration { kind: *b"hvcC", data: vec![1, 2, 3] },
//!         }],
//!         payload: vec![0xAA, 0xBB, 0xCC, 0xDD],
//!     }],
//!     groups: vec![],
//! };
//! let bytes = write(&img).unwrap();
//!
//! let container = HeifContainer::parse(&bytes).unwrap();
//! assert!(container.image().is_hevc_still());
//! assert_eq!(container.image().primary_item().id(), 1);
//! // Every byte is accounted for: the segments tile 0..len exactly.
//! assert_eq!(container.segments().first().unwrap().range.start, 0);
//! assert_eq!(container.segments().last().unwrap().range.end, bytes.len());
//! // No appended stream or trailer in a clean file.
//! assert!(container.appended_stream().is_none());
//! assert!(container.trailer().is_none());
//! ```
#![forbid(unsafe_code)]

mod container;
mod image;

pub use container::{HeifContainer, Segment, SegmentKind, UnknownBox, UnknownBoxLocation};
pub use image::{
    CleanAperture, ContentLightLevel, HeifImage, HeifItem, ItemKind, PixelAspectRatio,
    TransformativeProperty,
};
