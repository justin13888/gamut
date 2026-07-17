//! `gamut-tiff` — TIFF 6.0 (Tagged Image File Format) image encoder and decoder.
//!
//! TIFF is a *natively still-image* format: its Image File Directory (IFD) / tag structure **is**
//! the container, so this crate needs neither
//! [`gamut_isobmff`](https://crates.io/crates/gamut-isobmff) (AVIF/HEIC) nor
//! [`gamut_riff`](https://crates.io/crates/gamut-riff) (WebP). That IFD container core — the
//! byte-order header, field types/values, the IFD chain, and the offset-driven read/write spine —
//! is the shared [`gamut_ifd`](https://crates.io/crates/gamut-ifd) primitive (also the basis for
//! EXIF); this crate adds the codec on top and re-exports the structural types from its root so its
//! public API is unchanged. It further layers on the shared primitives: [`gamut_core`] (traits /
//! errors / typed pixel formats) and [`gamut_bitstream`] (LZW and CCITT bit coding). The
//! differencing predictor is TIFF-specific and lives in this crate;
//! the deferred colour-space work (YCbCr, CIE L\*a\*b\*) and JPEG-in-TIFF will bring back the
//! `gamut-color` and `gamut-dsp` edges additively when they land (see `STATUS.md`).
//!
//! The encoder and decoder are reachable through the umbrella crate's `tiff` feature. Everything
//! is implemented clean-slate from the TIFF 6.0 specification (`references/tiff/tiff6.pdf`,
//! Adobe/Aldus, Final — June 3 1992) and the BigTIFF extension (`references/tiff/bigtiff.html`)
//! rather than wrapping libtiff.
//!
//! Implementation in progress (see issue #107). The codec layer (photometric/predictor
//! semantics, strip/tile/multi-page layout over [`gamut_ifd::write`], [`tags`], the
//! compression schemes) and the baseline pixel path are in place: [`TiffEncoder`] writes 8-bit
//! grayscale/RGB/RGBA/CMYK, 1-bit bilevel, and 8-bit palette images (as strips or tiles) —
//! uncompressed, PackBits, LZW, or (for bilevel) Modified Huffman / Group 4 fax — and
//! [`TiffDecoder`] reads them back. Encoding takes a typed [`gamut_core::ImageRef`] via the
//! per-format [`gamut_core::EncodeImage`] impls, and decoding returns a [`gamut_core::ImageBuf`] via
//! [`gamut_core::DecodeImage`]. Both the classic 32-bit container and
//! **BigTIFF** (magic `43`, 64-bit offsets, for files past 4 GiB) are written and read: opt into
//! BigTIFF with [`TiffEncoder::with_big_tiff`], and the decoder detects the variant from the
//! header. The remaining compression schemes and colour modes land in subsequent phases.
#![forbid(unsafe_code)]

// Single canonical paths (the gamut-ifd v1 precedent): the implementation modules are private
// and the public surface is the crate-root re-export list below. `tags` is the one deliberate
// exception — a namespaced constants module, mirroring `gamut_ifd::tags`.
pub mod tags;

mod compression;
mod decoder;
mod deconstruct;
mod encoder;
mod ifd;
mod palette;
mod writer;

pub use compression::Compression;
pub use decoder::TiffDecoder;
pub use deconstruct::{Anomaly, DeconstructReport, Severity, UnknownTag, deconstruct};
pub use encoder::TiffEncoder;
// The structural IFD core lives in gamut-ifd; re-export the types a gamut-tiff user can touch —
// the read/write spine plus every type reachable from this crate's own public items
// (`DeconstructReport` exposes `CoverageReport`/`UnknownField`, `CoverageReport` exposes
// `Range`/`Overlap`, and `Ifd`'s accessors return `SubIfd`) — so no direct gamut-ifd dependency
// is ever needed to name them.
pub use gamut_ifd::{
    ByteOrder, CoverageReport, Field, FieldType, Ifd, Overlap, Range, SubIfd, TiffFile,
    UnknownField, Value, Variant, read, write,
};
pub use ifd::{PhotometricInterpretation, Predictor};
pub use palette::Palette8;
pub use writer::{write_image, write_image_tiled, write_multipage};
