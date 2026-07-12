//! `gamut-jpeg` — a spec-compliant **JPEG-1** (ISO/IEC 10918-1 | ITU-T T.81) still-image codec.
//!
//! JPEG-1 codes a continuous-tone image as 8×8 blocks transformed by the discrete cosine transform,
//! quantized, and entropy-coded, wrapped in a marker-delimited stream (SOI, tables, a frame header,
//! scan(s), EOI). This crate is built against the primary specifications, with clause citations in
//! the source:
//!
//! - **ITU-T T.81 | ISO/IEC 10918-1** — the core codec. Annex A (the DCT process: level shift
//!   §A.3.1, FDCT §A.3.3, quantization §A.3.4, DC prediction §A.3.5, zig-zag §A.3.6, MCU
//!   interleaving §A.2.3); Annex B (compressed-data formats and markers); Annex C (Huffman code
//!   generation); Annex F (§F.1.2 baseline Huffman entropy coding); Annex K (§K.1 quantization and
//!   §K.3 typical Huffman tables).
//! - **ITU-T T.871 | ISO/IEC 10918-5** — JFIF: the APP0 segment (§10.1) and the full-range BT.601
//!   YCbCr conversion (§7).
//!
//! # Scope
//!
//! This crate is a JPEG **encoder + decoder** (unlike the workspace's encoder-only PNG crate — JPEG
//! is a two-way format). The first phase ships the **baseline sequential DCT Huffman encoder**
//! (SOF0, 8-bit): grayscale and JFIF YCbCr with 4:4:4 / 4:2:2 / 4:2:0 subsampling, standard (Annex
//! K) tables, a quality→quantization mapping, and optional restart intervals. The sequential and
//! progressive decoders, a progressive encoder, and Adobe APP14 / CMYK handling land in later
//! phases (see `STATUS.md`); the public API is designed so they slot in without breaking changes.
//!
//! Out of scope (see `STATUS.md`): 12-bit precision, arithmetic coding, the lossless and
//! hierarchical processes, DNL, and the SPIFF/T.84/T.872 layers.
//!
//! # Oracle
//!
//! Correctness is proven differentially against **libjpeg-turbo** (a vendored, dev-only static
//! build, landing with the decoder phase), cross-checked against the vendored **T.873 reference
//! software**. Until the decoder lands, the encoder is pinned by byte-exact micro-goldens derived
//! from T.81 Annex F/K and a structural stream walker.
//!
//! # Example
//!
//! ```
//! use gamut_core::{Dimensions, EncodeImage, ImageRef, Rgb8};
//! use gamut_jpeg::{ChromaSubsampling, JpegEncoder};
//!
//! let (w, h) = (16, 16);
//! let rgb = vec![0u8; (w * h * 3) as usize];
//! let image = ImageRef::<Rgb8>::new(&rgb, Dimensions::new(w, h)?)?;
//! let mut jpeg = Vec::new();
//! JpegEncoder::new()
//!     .with_quality(85)
//!     .with_subsampling(ChromaSubsampling::Ycbcr420)
//!     .encode_image(image, &mut jpeg)?;
//! assert_eq!(&jpeg[..2], &[0xFF, 0xD8]); // SOI
//! assert_eq!(&jpeg[jpeg.len() - 2..], &[0xFF, 0xD9]); // EOI
//! # Ok::<(), gamut_core::Error>(())
//! ```
#![forbid(unsafe_code)]

mod bitwriter;
mod encoder;
mod huffman;
mod marker;
mod quant;
mod zigzag;

pub use encoder::{ChromaSubsampling, JpegEncoder};
pub use marker::DensityUnit;
