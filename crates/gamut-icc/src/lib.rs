//! `gamut-icc` — ICC color profile parsing and serialization.
//!
//! An ICC profile is the self-describing color-characterization blob embedded in images (the WebP
//! `ICCP` chunk, the AVIF/HEIF `colr` box of type `prof`, a JPEG `APP2` segment). Structurally it
//! is a 128-byte header, a tag table, then the tag element data the table points at — a flat,
//! offset-indexed binary format that needs neither the TIFF/IFD machinery nor XML, so this crate
//! depends only on [`gamut_core`].
//!
//! Layouts follow the ICC profile specification **ICC.1:2022** (profile version 4.4,
//! `references/icc`), which is equivalent to ISO 15076-1. Profile **v2** is still by far the most
//! common version embedded in real images and is supported for reading.
//!
//! Placeholder skeleton — implementation pending (see issue #34). The type declarations below
//! sketch the data model the implementation phases flesh out; no parsing/serialization exists yet.
#![forbid(unsafe_code)]

pub mod header;
pub mod profile;
pub mod reader;
pub mod tag_types;
pub mod tags;
pub mod writer;

pub use header::{ColorSpace, DeviceClass, ProfileHeader, ProfileVersion, RenderingIntent};
pub use profile::IccProfile;
pub use reader::IccReader;
pub use tag_types::TagType;
pub use tags::{KnownTag, TagEntry, TagSignature};
pub use writer::IccWriter;
