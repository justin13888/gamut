//! `gamut-metadata` — the unified image-metadata facade.
//!
//! Brings the per-format metadata crates — [`exif`], [`xmp`], [`icc`], and [`iptc`] — under one
//! [`Metadata`] model and one extract/embed surface. A container crate (e.g.
//! [`gamut_isobmff`](https://crates.io/crates/gamut-isobmff) for AVIF/HEIC,
//! [`gamut_riff`](https://crates.io/crates/gamut-riff) for WebP) locates the metadata payloads in a
//! file; this facade turns those payloads into typed models and back. It stays
//! **container-agnostic** — it consumes already-located [`MetadataBlock`]s and produces owned
//! [`EncodedMetadata`] blocks, never boxes or chunks — and **orchestration-only**: it holds the leaf
//! crates' types by value and delegates all parsing and serialization to them.
//!
//! # The model: one carrier, one field
//!
//! [`Metadata`] has exactly one field per genuinely distinct serialization a container holds —
//! [`exif`](Metadata::exif), [`xmp`](Metadata::xmp), [`icc`](Metadata::icc). **IPTC has no field of
//! its own:** IPTC Photo Metadata *is* XMP (properties in the `dc:`/`photoshop:`/`Iptc4xmp*`
//! namespaces), so it lives inside [`xmp`](Metadata::xmp), read back through the
//! [`Metadata::iptc`] lens. The one genuinely separate IPTC carrier — the legacy binary IIM block —
//! is reconciled *into* the XMP graph on [extraction](MetadataExtractor) (with a
//! [`ConflictPolicy`]) and projected back out only on request when [embedding](MetadataEmbedder).
//! Because each datum is stored once, the extract→embed→extract round-trip is a true equality.
//!
//! # Extensions: data with no carrier
//!
//! A downstream typed model is usually wider than what a still-image file can carry — sensor
//! geometry, container-level facts, structs it derives itself. [`Metadata::extensions`] is a
//! namespaced table for exactly that residue, so such a model survives
//! `their model → Metadata → their model` intact instead of being silently narrowed to three
//! carriers. Each entry is a [`MetadataExtension`]: a namespace the caller owns, a key, and a
//! value in the same TIFF/IFD [`Value`](exif::Value) model gamut's metadata crates already use.
//!
//! Two guarantees, deliberately distinct:
//!
//! - **Model round-trip.** Everything in a [`Metadata`] — carriers *and* extensions — survives
//!   being handed to another model and back. This is what extensions are for.
//! - **Carrier round-trip** (the keystone, unchanged). extract → embed → extract is still a true
//!   equality over [`exif`](Metadata::exif)/[`xmp`](Metadata::xmp)/[`icc`](Metadata::icc).
//!   Extensions take no part: extraction never produces one, and [`Metadata::encode`] drops them
//!   (or fails, under [`ExtensionPolicy::Reject`]).
//!
//! **Prefer a carrier whenever one exists** — only a carrier reaches the file. An unmodelled EXIF
//! tag, MakerNote included, already round-trips inside [`Metadata::exif`] because
//! [`Exif`](exif::Exif) retains the raw [`Ifd`](exif::Ifd); any property round-trips inside
//! [`Metadata::xmp`] because the XMP graph is open; an unmodelled ICC element round-trips inside
//! [`Metadata::icc`] as `TagData::Raw`. Reach for an extension only when no carrier can hold the
//! datum at all.
//!
//! ```
//! use gamut_metadata::{ExtensionPolicy, Metadata, MetadataEmbedder, MetadataError};
//! use gamut_metadata::exif::Value;
//!
//! // A downstream model parks a sensor fact no still-image carrier expresses.
//! let mut meta = Metadata::default();
//! meta.set_extension("com.example.raw", "WhiteLevel", Value::Long(vec![16_383]));
//! assert_eq!(
//!     meta.extension("com.example.raw", "WhiteLevel"),
//!     Some(&Value::Long(vec![16_383]))
//! );
//!
//! // Embedding cannot carry it — by default it is dropped...
//! assert_eq!(meta.encode()?, Metadata::default().encode()?);
//!
//! // ...and a caller that must not lose it silently can say so.
//! let rejected = MetadataEmbedder::new()
//!     .extension_policy(ExtensionPolicy::Reject)
//!     .embed(&meta);
//! assert!(matches!(
//!     rejected,
//!     Err(MetadataError::UnembeddableExtension { .. })
//! ));
//! # Ok::<(), gamut_metadata::MetadataError>(())
//! ```
//!
//! # Quick start
//!
//! ```
//! use gamut_metadata::{Metadata, MetadataBlock};
//! use gamut_metadata::xmp::{WellKnownNs, XmpMeta};
//!
//! // A container crate located an XMP packet; build one here for the example.
//! let mut graph = XmpMeta::new();
//! graph.set_text(WellKnownNs::Photoshop.uri(), "City", "Kyoto");
//! let packet = graph.to_packet();
//!
//! // Extract a unified model from the located blocks...
//! let meta = Metadata::from_blocks(&[MetadataBlock::Xmp(&packet)])?;
//! assert_eq!(meta.iptc().unwrap().city(), Some("Kyoto"));
//!
//! // ...then serialize back to per-carrier byte blocks for a container to embed.
//! let blocks = meta.encode()?;
//! assert!(blocks.xmp.is_some());
//! # Ok::<(), gamut_metadata::MetadataError>(())
//! ```
#![forbid(unsafe_code)]

pub mod embed;
pub mod error;
pub mod extension;
pub mod extract;
pub mod metadata;
pub mod source;

// Re-export the per-format crates so consumers reach everything through one entry point.
pub use embed::{EncodedMetadata, ExtensionPolicy, MetadataEmbedder};
pub use error::{MetadataError, Result};
pub use extension::{MetadataExtension, RESERVED_NAMESPACE_PREFIX};
pub use extract::MetadataExtractor;
pub use gamut_exif as exif;
pub use gamut_icc as icc;
pub use gamut_iptc as iptc;
// Surface the IPTC reconciliation knobs at the facade level so callers configure extraction without
// reaching into the `iptc` submodule.
pub use gamut_iptc::{ConflictPolicy, FieldConflict};
pub use gamut_xmp as xmp;
pub use metadata::Metadata;
pub use source::MetadataBlock;
