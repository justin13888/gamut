//! `gamut-iptc` — IPTC photo metadata parsing and serialization.
//!
//! IPTC photo metadata exists in two forms, and this crate covers both:
//!
//! - **Legacy IIM** (Information Interchange Model) — a binary record/dataset stream, in practice
//!   embedded inside a Photoshop Image Resource Block (resource id `0x0404`, an `8BIM` block) within
//!   a JPEG `APP13` segment. Implemented in [`iim`] (the dataset codec), [`irb`] (the `8BIM`
//!   carrier), and [`charset`] (the dataset 1:90 coded character set).
//! - **IPTC Photo Metadata** (Core + Extension) — the modern standard, serialized **as XMP**.
//!   Modelled in [`photo_metadata`] on top of [`gamut_xmp`]'s property graph.
//!
//! The two overlap heavily; reconciling them (which value wins when both carry the same datum) is
//! the crate's keystone, in [`reconcile`].
//!
//! Standards: **IPTC-IIM 4.2** and the **IPTC Photo Metadata Standard** (`references/iptc`).
//!
//! # Reading and writing legacy IIM
//!
//! ```
//! use gamut_iptc::{IimBlock, IimCharset, IimDataSet, IptcReader, IptcWriter};
//!
//! // Build some descriptive datasets and serialize them into an 8BIM resource stream.
//! let block = IimBlock {
//!     datasets: vec![
//!         IimDataSet { record: 2, dataset: 0, data: vec![0, 4] }, // Record Version = 4
//!         IimDataSet { record: 2, dataset: 25, data: b"sky".to_vec() }, // Keywords
//!     ],
//! };
//! let irb = IptcWriter::new().write_irb(&block)?;
//!
//! // Read it back and decode a text value with the stream's charset.
//! let parsed = IptcReader::new().read_irb(&irb)?.expect("0x0404 resource present");
//! let charset = IimCharset::detect(&parsed)?;
//! assert_eq!(charset.decode(&parsed.datasets[1].data)?, "sky");
//! # Ok::<(), gamut_core::Error>(())
//! ```
//!
//! # Scope
//!
//! gamut-iptc is the IPTC *semantics* layer. For the modern path it operates on an in-memory XMP
//! property graph; parsing and serializing the XMP packet bytes is [`gamut_xmp`]'s responsibility.
//! Exotic ISO 2022 character sets beyond Latin-1 and UTF-8 are reported as
//! [`gamut_core::Error::Unsupported`] rather than mis-decoded (see [`charset`]).
#![forbid(unsafe_code)]

pub mod charset;
pub mod iim;
pub mod irb;
pub mod photo_metadata;
pub mod reader;
pub mod reconcile;
pub mod writer;

pub use charset::IimCharset;
pub use iim::{IimBlock, IimDataSet, IimFieldKind, IimRecord, IimTagInfo, tag_info};
pub use irb::{IrbBlock, PhotoshopIrb};
pub use photo_metadata::PhotoMetadata;
pub use reader::IptcReader;
pub use reconcile::IimXmpReconciler;
pub use writer::IptcWriter;
