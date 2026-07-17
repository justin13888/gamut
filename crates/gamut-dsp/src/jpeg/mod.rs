//! JPEG-1 transform kernels (ITU-T T.81 | ISO/IEC 10918-1, Annex A).
//!
//! The DCT-based JPEG processes transform each 8×8 block of level-shifted samples with the 2-D
//! discrete cosine transform of T.81 §A.3.3:
//! - the forward DCT ([`fdct8x8`], the encoder transform), and
//! - the inverse DCT ([`idct8x8`], the decoder transform).
//!
//! Both operate in place on a raster-order (row-major / natural) `[i32; 64]` block — the sample
//! and coefficient orientation of T.81 Figure A.4 — and are separable `f64` evaluations of the
//! *informative* §A.3.3 equations with a single final rounding. The surrounding pipeline stages
//! that §A.3 layers around the transform — the §A.3.1 level shift, §A.3.4 quantization, §A.3.5
//! differential DC, and §A.3.6 zig-zag reordering — are the `gamut-jpeg` codec's responsibility,
//! not this kernel module's.
//!
//! Per T.81 §A.3.3 these ideal equations "cannot be represented with perfect accuracy by any real
//! implementation"; conformance to the normative accuracy bounds of T.83 (ISO/IEC 10918-2) is a
//! codec-level concern. Fast fixed-point DCT kernels are deferred (issue #28 scope note): this
//! module favours a legible, spec-faithful reference implementation.

mod dct;

pub use dct::{fdct8x8, idct8x8};
