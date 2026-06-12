//! `gamut-metadata` — the unified image-metadata facade.
//!
//! Brings the per-format metadata crates — [`exif`], [`xmp`], [`icc`], and [`iptc`] — under one
//! [`Metadata`] model and one extract /
//! embed surface. The container crates ([`gamut_isobmff`](https://crates.io/crates/gamut-isobmff)
//! for AVIF/HEIC, [`gamut_riff`](https://crates.io/crates/gamut-riff) for WebP) already locate the
//! metadata payloads in a file; this facade turns those payloads into typed models and back,
//! keeping itself **container-agnostic** — it takes already-located [`MetadataBlock`]s, never boxes
//! or chunks.
//!
//! This is the surface the format crates (`gamut-avif`/`gamut-webp`/`gamut-heic`) will consume for
//! their metadata milestones; wiring those edges is a later step, out of scope for this scaffold.
//!
//! Placeholder skeleton — implementation pending (see issue #34). The type declarations below sketch
//! the unified model and the extract/embed entry points; no logic exists yet.
#![forbid(unsafe_code)]

pub mod embed;
pub mod extract;
pub mod metadata;
pub mod source;

// Re-export the per-format crates so consumers reach everything through one entry point.
pub use gamut_exif as exif;
pub use gamut_icc as icc;
pub use gamut_iptc as iptc;
pub use gamut_xmp as xmp;

pub use embed::MetadataEmbedder;
pub use extract::MetadataExtractor;
pub use metadata::Metadata;
pub use source::MetadataBlock;
