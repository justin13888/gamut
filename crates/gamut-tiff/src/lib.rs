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
//! errors), [`gamut_color`] (photometric & pixel formats, incl. palette / CMYK / YCbCr /
//! CIE L\*a\*b\*), [`gamut_dsp`] (the differencing predictor and the DCT used by JPEG-in-TIFF), and
//! [`gamut_bitstream`] (LZW and CCITT bit coding).
//!
//! The encoder and decoder are reachable through the umbrella crate's `tiff` feature. Everything
//! is implemented clean-slate from the TIFF 6.0 specification (`references/tiff/tiff6.pdf`,
//! Adobe/Aldus, Final — June 3 1992) and the BigTIFF extension (`references/tiff/bigtiff.html`)
//! rather than wrapping libtiff.
//!
//! Implementation in progress (see issue #107). The codec layer ([`ifd`] photometric/predictor
//! semantics, [`writer`] strip/tile/multi-page layout over [`gamut_ifd::write`], [`tags`],
//! [`compression`]) and the baseline pixel path are in place: [`TiffEncoder`] writes 8-bit
//! grayscale/RGB/RGBA/CMYK, 1-bit bilevel, and 8-bit palette images (as strips or tiles) —
//! uncompressed, PackBits, LZW, or (for bilevel) Modified Huffman / Group 4 fax — and
//! [`TiffDecoder`] reads them back (both implement the
//! [`gamut_core::Encoder`]/[`gamut_core::Decoder`] traits). Both the classic 32-bit container and
//! **BigTIFF** (magic `43`, 64-bit offsets, for files past 4 GiB) are written and read: opt into
//! BigTIFF with [`TiffEncoder::with_big_tiff`], and the decoder detects the variant from the
//! header. The remaining compression schemes and colour modes land in subsequent phases.
#![forbid(unsafe_code)]

pub mod compression;
pub mod decoder;
pub mod encoder;
pub mod ifd;
pub mod tags;
pub mod writer;

pub use compression::Compression;
pub use decoder::TiffDecoder;
pub use encoder::TiffEncoder;
// The structural IFD core lives in gamut-ifd; re-export it so gamut-tiff's public API is unchanged.
pub use gamut_ifd::{ByteOrder, Field, FieldType, Ifd, TiffFile, Value, Variant, read, write};
pub use ifd::{PhotometricInterpretation, Predictor};
pub use writer::{write_image, write_image_tiled, write_multipage};
