//! HEIC/HEIF still-image **container decoder** — HEVC intra image items in an ISOBMFF container.
//!
//! This crate is decode-side and, for issue #238, covers the container, NAL, and **decode-pipeline**
//! layers: it parses a HEIF/HEIC file into a byte-exact representation and a role-typed semantic
//! view, decodes the `hvcC` HEVCDecoderConfigurationRecord into a typed [`HevcConfig`],
//! splits/classifies the coded item payload into NAL units, and — around a pluggable
//! [`HevcDecoder`] — resolves item derivation, colour, alpha, and the transformative properties to
//! deliver planar samples or a presentation-ready [`ImageBuf<Rgba8>`](gamut_core::ImageBuf). It does
//! **not** encode HEIF (gamut is decode-only for this format, see `references/heif`), and it does not
//! itself interpret the coded HEVC RBSP payloads — that codestream decode is the [`HevcDecoder`] the
//! caller plugs in (HEVC-intra pixel reconstruction is codec scope, issue #18).
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
//! # HEVC configuration & NAL layer
//!
//! [`HevcConfig::parse`] decodes an `hvcC` record ([`references/heif`](../heif) §1) into typed
//! profile/tier/level, chroma/bit-depth, and the parameter-set [`arrays`](HevcConfig::arrays);
//! [`HeifItem::hevc_config`] reaches it from a coded item. [`iter_nal_units`] splits a length-prefixed
//! `hvc1`/`hev1` payload (§2), [`NalHeader::parse`] reads the two-byte NAL header and
//! [`NalUnitType`] classifies it (§3), [`HevcConfig::validate_still_payload`] enforces the still-image
//! IRAP constraint, and [`HevcConfig::annex_b`] converts a payload to a start-coded NAL stream for a
//! downstream decoder.
//!
//! # Decode pipeline
//!
//! [`HevcDecoder`] is the pluggable HEVC-intra codestream hook a caller implements (typically
//! wrapping a platform decoder). Around it, [`HeifImage::decode_item_planar`] returns raw
//! [`DecodedFrame`] samples (resolving `iden`/`grid` derivation), and
//! [`HeifImage::decode_item_rgba8`] / [`HeifImage::decode_primary_rgba8`] add colour conversion,
//! alpha merge, `iovl` compositing, and the transformative properties to produce an
//! [`ImageBuf<Rgba8>`](gamut_core::ImageBuf). See the [`HevcDecoder`] docs for the FFI rationale and
//! the exact colour policy.
//!
//! # Deferred to later slices
//!
//! Wiring the decoded Exif/XMP bytes through `gamut-exif`/`gamut-xmp`, and the libheif differential
//! oracle. Image *sequences* (`msf1`/`hevc`/`hevx` tracks) are permanently out of scope (gamut is
//! image-first). See this crate's `STATUS.md`.
//!
//! # Example
//!
//! Parse a HEIF file, then drive the full decode through a caller-supplied [`HevcDecoder`]. The stub
//! here returns a solid-gray monochrome frame; a real decoder wraps a platform HEVC-intra decoder
//! (libde265, VideoToolbox, …).
//!
//! ```
//! use gamut_heic::{ChromaFormat, DecodedFrame, HeifContainer, HevcConfig, HevcDecoder};
//! use gamut_core::Result;
//! use gamut_isobmff::{IsoBmffImage, Item, Property, PropertyKind, write};
//!
//! // A stub HEVC decoder: ignores the codestream and returns a 2x2 solid-gray monochrome frame.
//! struct GrayStub;
//! impl HevcDecoder for GrayStub {
//!     fn decode_intra(&mut self, _config: &HevcConfig, _payload: &[u8]) -> Result<DecodedFrame> {
//!         DecodedFrame::new(2, 2, 8, ChromaFormat::Monochrome, vec![128; 4], vec![], vec![])
//!     }
//! }
//!
//! # fn hvcc() -> Vec<u8> {
//! #     // Minimal valid hvcC: 23-byte header, lengthSizeMinusOne = 3, numOfArrays = 0.
//! #     let mut v = vec![0u8; 23];
//! #     v[0] = 1; // configurationVersion
//! #     v[21] = 0b0000_0011; // ... | lengthSizeMinusOne = 3
//! #     v[22] = 0; // numOfArrays
//! #     v
//! # }
//! // A minimal HEVC still: one hvc1 item with an hvcC config and a single IRAP (IDR) NAL payload.
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
//!             kind: PropertyKind::CodecConfiguration { kind: *b"hvcC", data: hvcc() },
//!         }],
//!         // 4-byte length prefix + an IDR_W_RADL NAL (type 19: header byte 0x26).
//!         payload: vec![0x00, 0x00, 0x00, 0x03, 0x26, 0x01, 0xDD],
//!     }],
//!     groups: vec![],
//! };
//! let bytes = write(&img).unwrap();
//!
//! let container = HeifContainer::parse(&bytes).unwrap();
//! assert!(container.image().is_hevc_still());
//! assert_eq!(container.image().primary_item().id(), 1);
//!
//! // Decode the primary image to RGBA through the stub decoder.
//! let rgba = container.image().decode_primary_rgba8(&mut GrayStub).unwrap();
//! assert_eq!((rgba.width(), rgba.height()), (2, 2));
//! assert_eq!(rgba.as_samples()[3], 255); // opaque (no alpha auxiliary)
//! ```
#![forbid(unsafe_code)]

mod container;
mod decode;
mod hvcc;
mod image;
mod nal;

pub use container::{HeifContainer, Segment, SegmentKind, UnknownBox, UnknownBoxLocation};
pub use decode::{DecodedFrame, HevcDecoder};
pub use hvcc::{ChromaFormat, HevcConfig, NalArray};
pub use image::{
    CleanAperture, ContentLightLevel, HeifImage, HeifItem, ItemKind, PixelAspectRatio,
    TransformativeProperty,
};
pub use nal::{NalHeader, NalUnitIter, NalUnitType, iter_nal_units};
