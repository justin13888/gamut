//! `gamut-exif` — EXIF image metadata parsing and serialization.
//!
//! An EXIF blob is an `Exif\0\0` marker followed by a TIFF stream (the JPEG `APP1` payload, the
//! WebP `EXIF` chunk, the AVIF/HEIF `Exif` item). Its structure is a chain of IFDs — the 0th
//! (primary image) and 1st (thumbnail) directories, plus the Exif, GPS, and Interoperability
//! sub-IFDs reached through pointer tags — so this crate builds on the shared
//! [`gamut_ifd`](https://crates.io/crates/gamut-ifd) TIFF/IFD core and adds the EXIF tag
//! dictionary, value interpretation, and the sub-IFD layout on top.
//!
//! Tags and semantics follow **Exif 3.0** (CIPA DC-008-2024; `references/exif`), with Exif 2.32
//! retained for legacy compatibility. The long-term goal (issue #34) is exiftool-class tag
//! coverage, including the vendor-specific MakerNote dialects.
//!
//! Placeholder skeleton — implementation pending. The type declarations below sketch the data model;
//! no parsing/serialization exists yet.
#![forbid(unsafe_code)]

pub mod exif;
pub mod gps;
pub mod makernote;
pub mod reader;
pub mod tags;
pub mod value;
pub mod writer;

pub use exif::Exif;
pub use gps::{GpsCoordinate, GpsInfo, GpsReference};
pub use makernote::{MakerNote, MakerNoteVendor};
pub use reader::ExifReader;
pub use tags::{ExifTag, IfdKind};
pub use value::ExifValue;
pub use writer::ExifWriter;
