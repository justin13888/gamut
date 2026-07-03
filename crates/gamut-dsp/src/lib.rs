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
//!
//! # Contract
//!
//! Every function is pure, total, deterministic math on caller-provided values — no
//! allocation, no dependencies, `#![forbid(unsafe_code)]`, and no `Result`s: nothing here is
//! data-dependent fallible. Semantic preconditions on configuration parameters (`n`, `r`,
//! `bits`, `mu`, `den` — encoder configuration, never untrusted data) are asserts documented
//! under `# Panics`; arithmetic headroom limits are documented per function and guarded by
//! Rust's debug overflow checks.
//!
//! ```
//! use gamut_dsp::av1::{forward_wht4x4, inverse_wht4x4};
//! use gamut_dsp::math::round_div_nearest;
//!
//! // The lossless 4×4 Walsh–Hadamard pair round-trips exactly.
//! let residual = [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
//! let coeffs = forward_wht4x4(&residual);
//! assert_eq!(inverse_wht4x4(&coeffs), residual);
//!
//! // The shared forward-quantize rounding divides to the nearest level, ties away from zero.
//! assert_eq!(round_div_nearest(-10, 4), -3);
//!
//! // µ-law quantization: the center index of the odd level count is exactly 0.0, both ways.
//! assert_eq!(gamut_dsp::mulaw::quantize(0.0, 5, 5.0), 15);
//! assert_eq!(gamut_dsp::mulaw::dequantize(15, 5, 5.0), 0.0);
//! ```
#![forbid(unsafe_code)]

pub mod av1;
pub mod math;
pub mod mulaw;
