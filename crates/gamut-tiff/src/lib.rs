//! `gamut-tiff` — TIFF 6.0 (Tagged Image File Format) image encoder and decoder.
//!
//! TIFF is a *natively still-image* format: its Image File Directory (IFD) / tag structure **is**
//! the container, so this crate is self-contained and depends on no `gamut` container crate
//! (unlike AVIF/HEIC on [`gamut_isobmff`](https://crates.io/crates/gamut-isobmff) or WebP on
//! [`gamut_riff`](https://crates.io/crates/gamut-riff)). It layers only on the shared primitives:
//! [`gamut_core`] (traits / errors), [`gamut_color`] (photometric & pixel formats, incl.
//! palette / CMYK / YCbCr / CIE L\*a\*b\*), [`gamut_dsp`] (the differencing predictor and the DCT
//! used by JPEG-in-TIFF), and [`gamut_bitstream`] (LZW and CCITT bit coding).
//!
//! The encoder and decoder are reachable through the umbrella crate's `tiff` feature. Everything
//! is implemented clean-slate from the TIFF 6.0 specification (`references/tiff/tiff6.pdf`,
//! Adobe/Aldus, Final — June 3 1992) and the BigTIFF extension (`references/tiff/bigtiff.html`)
//! rather than wrapping libtiff.
//!
//! Implementation in progress (see issue #107). The container layer ([`ifd`], [`reader`],
//! [`writer`], [`tags`]) and the baseline pixel path are in place: [`TiffEncoder`] writes 8-bit
//! grayscale/RGB/RGBA/CMYK, 1-bit bilevel, and 8-bit palette images (as strips or tiles) —
//! uncompressed, PackBits, LZW, or (for bilevel) Modified Huffman / Group 4 fax ([`compression`]) —
//! and [`TiffDecoder`] reads them back (both implement the
//! [`gamut_core::Encoder`]/[`gamut_core::Decoder`] traits). Both the classic 32-bit container and
//! **BigTIFF** (magic `43`, 64-bit offsets, for files past 4 GiB) are written and read: opt into
//! BigTIFF with [`TiffEncoder::with_big_tiff`], and the decoder detects the variant from the
//! header. The remaining compression schemes and colour modes land in subsequent phases.
#![forbid(unsafe_code)]

pub mod compression;
pub mod decoder;
pub mod encoder;
pub mod ifd;
pub mod reader;
pub mod tags;
pub mod writer;

pub use compression::Compression;
pub use decoder::TiffDecoder;
pub use encoder::TiffEncoder;
pub use ifd::{
    ByteOrder, Field, FieldType, Ifd, PhotometricInterpretation, Predictor, Value, Variant,
};
pub use reader::{TiffFile, read};
pub use writer::{write, write_image, write_image_tiled, write_multipage};
