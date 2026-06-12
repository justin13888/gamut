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
//! §2). The type names here intentionally mirror `gamut-tiff`'s current structural types so the
//! codec can later adopt this crate as a near-zero-diff refactor (see `STATUS.md`).
//!
//! Placeholder skeleton — implementation pending (see issue #34). The type declarations below sketch
//! the data model the implementation phases flesh out; no reading/writing logic exists yet.
#![forbid(unsafe_code)]

pub mod byte_order;
pub mod entry;
pub mod reader;
pub mod types;
pub mod value;
pub mod writer;

pub use byte_order::ByteOrder;
pub use entry::{Field, Ifd, TiffHeader};
pub use reader::IfdReader;
pub use types::FieldType;
pub use value::Value;
pub use writer::IfdWriter;
