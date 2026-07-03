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
pub mod extract;
pub mod metadata;
pub mod source;

// Re-export the per-format crates so consumers reach everything through one entry point.
pub use embed::{EncodedMetadata, MetadataEmbedder};
pub use error::{MetadataError, Result};
pub use extract::MetadataExtractor;
pub use gamut_exif as exif;
pub use gamut_icc as icc;
pub use gamut_iptc as iptc;
pub use gamut_xmp as xmp;
// Surface the IPTC reconciliation knobs at the facade level so callers configure extraction without
// reaching into the `iptc` submodule.
pub use gamut_iptc::{ConflictPolicy, FieldConflict};
pub use metadata::Metadata;
pub use source::MetadataBlock;
