//! Shared digital signal processing kernels for the gamut codecs.
//!
//! The crate is one module per spec family, plus the shared integer vocabulary:
//! - [`av1`] — the AV1 §7.13.2 transform kernels: the 1-D DCT / ADST / identity kernels and the
//!   lossless 4×4 Walsh–Hadamard block pair,
//! - [`math`] — the small cross-codec integer arithmetic primitives: the AV1 §4.7 rounding and
//!   clamp operations and the forward-quantize rounding shared by the AV1 and VP8 encoders, and
//! - [`mulaw`] — µ-law companding and odd-level quantization.
//!
//! Nothing lives at the crate root, so future spec families (JPEG, JPEG XL, AV2, …) land as new
//! sibling modules without ever colliding with an existing name.
#![forbid(unsafe_code)]

pub mod av1;
pub mod math;
pub mod mulaw;
