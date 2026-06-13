//! `gamut-ifd` — the TIFF Image File Directory (IFD) container core.
//!
//! TIFF's structural spine — an 8-byte byte-order header (II/MM mark + magic `42` + offset of the
//! first IFD) followed by a chain of IFDs, each a list of 12-byte tag entries holding values inline
//! (when ≤ 4 bytes) or at a file offset — is **not** unique to the TIFF image codec. **EXIF is a
//! constrained profile of exactly this structure** (an `Exif\0\0` marker then a TIFF stream), so
//! this crate factors the IFD container out as a shared primitive that both
//! [`gamut-exif`](https://crates.io/crates/gamut-exif) (issue #34) and
//! [`gamut-tiff`](https://crates.io/crates/gamut-tiff) (issue #107) build on. It models only the
//! structure (byte order, field types, values, IFD chains, offset layout) — never pixels,
//! compression, or photometry, which stay in the codec.
//!
//! Structure follows **TIFF 6.0** (`references/tiff/tiff6.pdf`, Adobe/Aldus, Final — June 3 1992,
//! §2). [`read`] / [`read_header`] parse a stream into a [`TiffFile`]; [`write`] serialises one
//! back, laying out the IFD chain and out-of-line value pool with the two-pass offset machinery.
//!
//! ## BigTIFF
//!
//! The `bigtiff` cargo feature adds BigTIFF support (`references/tiff/bigtiff.html`): the
//! [`Variant::Big`] container with 64-bit offsets/counts and the [`FieldType::Long8`] /
//! `SLong8` / `Ifd8` field types. It is additive and off by default, so classic-only consumers
//! (e.g. EXIF metadata) stay lean; the TIFF codec enables it.
//!
//! ```
//! use gamut_ifd::{ByteOrder, Ifd, TiffFile, Value, Variant, read, write};
//!
//! let mut ifd = Ifd::new();
//! ifd.set(256, Value::Short(vec![640])); // ImageWidth
//! ifd.set(257, Value::Short(vec![480])); // ImageLength
//! let file = TiffFile { order: ByteOrder::LittleEndian, variant: Variant::Classic, ifds: vec![ifd] };
//! let bytes = write(&file);
//! assert_eq!(read(&bytes).unwrap(), file);
//! ```
#![forbid(unsafe_code)]

pub mod byte_order;
pub mod entry;
pub mod reader;
pub mod types;
pub mod value;
pub mod writer;

pub use byte_order::ByteOrder;
pub use entry::{Field, Ifd, SubIfd, Variant};
pub use reader::{TiffFile, read, read_header, read_ifd_at};
pub use types::FieldType;
pub use value::Value;
pub use writer::write;
