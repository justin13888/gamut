//! `gamut-jpeg` — a spec-compliant **JPEG-1** (ISO/IEC 10918-1 | ITU-T T.81) still-image codec.
//!
//! JPEG-1 codes a continuous-tone image as 8×8 blocks transformed by the discrete cosine transform,
//! quantized, and entropy-coded, wrapped in a marker-delimited stream (SOI, tables, a frame header,
//! scan(s), EOI). This crate is built against the primary specifications, with clause citations in
//! the source:
//!
//! - **ITU-T T.81 | ISO/IEC 10918-1** — the core codec. Annex A (the DCT process: level shift
//!   §A.3.1, FDCT §A.3.3, quantization §A.3.4, DC prediction §A.3.5, zig-zag §A.3.6, MCU
//!   structure §A.2 / interleaving §A.2.3, point transform §A.4); Annex B (compressed-data formats
//!   and markers, incl. the progressive scan-header fields §B.2.3); Annex C (Huffman code
//!   generation); Annex F (§F.1.2 baseline Huffman encoding, §F.2 the `DECODE` / `RECEIVE` /
//!   `EXTEND` decoding procedures); Annex G (the progressive DCT mode: spectral selection and
//!   successive approximation, §G.1.1–G.1.2 encoding models and §G.2 decoding); Annex K (§K.1
//!   quantization, §K.2 optimized Huffman-table construction for the progressive encoder, and §K.3
//!   typical Huffman tables).
//! - **ITU-T T.871 | ISO/IEC 10918-5** — JFIF: the APP0 segment (§10.1) and the full-range BT.601
//!   YCbCr conversion (§7).
//! - **Adobe Technical Note #5116** — the APP14 "Adobe" colour-transform marker (RGB / YCbCr / YCCK).
//!
//! # Scope
//!
//! This crate is a JPEG **encoder + decoder** (unlike the workspace's encoder-only PNG crate — JPEG
//! is a two-way format). It ships a **baseline (SOF0) sequential and progressive (SOF2) 8-bit DCT
//! Huffman encoder**, and a **decoder for the sequential and progressive processes**. The encoder
//! writes grayscale and JFIF YCbCr with 4:4:4 / 4:2:2 / 4:2:0 subsampling, standard (Annex K) tables,
//! a quality→quantization mapping, and optional restart intervals; [`JpegEncoder::with_progressive`]
//! selects the progressive process (Annex G), which uses libjpeg's frozen `jpeg_simple_progression`
//! scan script with optimized per-scan Huffman tables (Annex K.2) and produces the same quantized
//! coefficients — hence the same decoded image — as the baseline encoding. The [`JpegDecoder`] reads
//! any spec-valid sequential **or progressive** stream — grayscale, YCbCr, RGB, and CMYK/YCCK (via
//! the JFIF APP0 / Adobe APP14 hints), interleaved or non-interleaved scans, spectral selection and
//! successive approximation, restart intervals, and (for sequential frames) DNL-defined heights —
//! presenting it as `Rgb8`, `Gray8`, or `Cmyk8`.
//!
//! Out of scope (see `STATUS.md`): 12-bit precision, arithmetic coding, and the lossless and
//! hierarchical processes, and the SPIFF/T.84/T.872 layers.
//!
//! # Oracle
//!
//! Correctness is proven differentially against **libjpeg-turbo** (a vendored, dev-only static
//! build; see `tests/oracle.rs`), cross-checked against the vendored **T.873 reference software**.
//! The gate runs both directions: gamut encodes → libjpeg-turbo decodes → matches the source within
//! the lossy tolerance, and libjpeg-turbo encodes → gamut decodes → matches libjpeg-turbo's own
//! decode of the same stream. The encoder is additionally pinned by byte-exact micro-goldens derived
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
mod decoder;
mod encoder;
mod huffman;
mod marker;
mod progressive;
mod quant;
mod scan;
mod syntax;
mod zigzag;

pub use decoder::{JpegDecoder, JpegInfo, JpegProcess, info};
pub use encoder::{ChromaSubsampling, JpegEncoder};
pub use marker::DensityUnit;
