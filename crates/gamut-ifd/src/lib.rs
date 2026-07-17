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
//! §2). [`read`] / [`read_header`] parse a stream into a [`TiffFile`]; [`write()`] serialises one
//! back, laying out the IFD chain and out-of-line value pool with the two-pass offset machinery.
//! [`read_tree`] is `write`'s inverse over sub-IFD trees (given the pointer tags — the
//! well-known structural ones live in [`tags`]), and the `*_with_coverage` readers thread
//! byte-range accounting ([`Coverage`]) for strict archival decoding.
//!
//! ## Streaming
//!
//! The slice readers want the whole file in memory; a multi-hundred-MB camera RAW does not fit
//! that shape, and its directory structure is kilobytes. [`IfdReader`] walks the same structure
//! lazily over any [`ReadAt`] source — a slice, a [`StreamSource`] (`Read + Seek`, e.g. a
//! file), or a [`Rebased`] offset-shifted view, which is the **maker-note primitive**: vendor
//! mini-IFDs address their values relative to the note start (or the enclosing TIFF header),
//! so the note is parsed as a directory over a rebased view of the same source:
//!
//! ```
//! use gamut_ifd::{ByteOrder, Ifd, IfdReader, ReadAt, TiffFile, Value, Variant, write};
//!
//! // A "maker note": a little TIFF stream embedded at offset 1000 of a larger file, its
//! // internal offsets relative to its own start.
//! let mut note = Ifd::new();
//! note.set(1, Value::Ascii("vendor mode".to_owned()));
//! let note_bytes = write(&TiffFile {
//!     order: ByteOrder::LittleEndian,
//!     variant: Variant::Classic,
//!     ifds: vec![note.clone()],
//! }).unwrap();
//! let mut file = vec![0u8; 1000];
//! file.extend_from_slice(&note_bytes);
//!
//! let mut reader = IfdReader::open((&file[..]).rebased(1000)).unwrap();
//! let raw = reader.read_ifd(reader.first_ifd_offset()).unwrap(); // one directory body
//! let value = reader.value(raw.entry(1).unwrap()).unwrap(); // one value, on demand
//! assert_eq!(value, Value::Ascii("vendor mode".to_owned()));
//! ```
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
//! let bytes = write(&file).unwrap();
//! assert_eq!(read(&bytes).unwrap(), file);
//! ```
#![forbid(unsafe_code)]

mod byte_order;
mod coverage;
mod entry;
mod reader;
mod segment;
mod source;
mod stream;
pub mod tags;
mod track;
mod types;
mod value;
mod writer;

pub use byte_order::ByteOrder;
pub use coverage::{Coverage, CoverageReport, Overlap, UnknownField};
pub use entry::{Field, Ifd, SubIfd, Variant};
pub use reader::{
    TiffFile, read, read_audited, read_header, read_ifd_at, read_ifd_at_with_coverage, read_tree,
    read_with_coverage,
};
pub use segment::{
    Claim, Conflict, DataLabel, Range, Segment, SegmentMap, SegmentReport, SharedSpan, SpanKind,
};
pub use source::{ReadAt, Rebased, StreamSource};
pub use stream::{IfdChain, IfdReader, RawEntry, RawIfd};
pub use track::{ReadLedger, Tracked};
pub use types::FieldType;
pub use value::{UnknownValue, Value};
pub use writer::{PinnedSpan, WriteOptions, align_word, write, write_with};
