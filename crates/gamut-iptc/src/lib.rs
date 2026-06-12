//! `gamut-iptc` — IPTC photo metadata parsing and serialization.
//!
//! IPTC photo metadata exists in two forms, and this crate covers both:
//!
//! - **Legacy IIM** (Information Interchange Model) — a binary record/dataset stream, in practice
//!   embedded inside a Photoshop Image Resource Block (resource id `0x0404`, an `8BIM` block) within
//!   a JPEG `APP13` segment. Modelled in [`iim`] and [`irb`].
//! - **IPTC Photo Metadata** (Core + Extension) — the modern standard, serialized **as XMP**.
//!   Modelled in [`photo_metadata`] on top of [`gamut_xmp`]'s property graph.
//!
//! The two overlap heavily; reconciling them (which value wins when both carry the same datum) is
//! the crate's keystone, in [`reconcile`].
//!
//! Standards: **IPTC-IIM 4.2** and the **IPTC Photo Metadata Standard** (`references/iptc`).
//!
//! Placeholder skeleton — implementation pending (see issue #34). The type declarations below sketch
//! the data model; no parsing/serialization exists yet.
#![forbid(unsafe_code)]

pub mod iim;
pub mod irb;
pub mod photo_metadata;
pub mod reader;
pub mod reconcile;
pub mod writer;

pub use iim::{IimDataSet, IimRecord, IimTag};
pub use irb::{IrbBlock, PhotoshopIrb};
pub use photo_metadata::PhotoMetadata;
pub use reader::IptcReader;
pub use reconcile::IimXmpReconciler;
pub use writer::IptcWriter;
