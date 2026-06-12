//! Shared digital signal processing routines for the gamut codecs.
//!
//! Provides the AV1 1-D transform kernels:
//! - the lossless 4×4 Walsh–Hadamard pair ([`fwht4x4`] / [`iwht4x4`], AV1 §7.13.2.10),
//! - the discrete cosine transform pair ([`forward_dct`] / [`inverse_dct`], AV1 §7.13.2.2–.3),
//! - the asymmetric discrete sine pair ([`forward_adst`] / [`inverse_adst`], AV1 §7.13.2.4–.9 —
//!   DST-VII at size 4, DST-IV at 8/16), with the FLIPADST flip ([`flip_in_place`]), and
//! - the identity transforms ([`forward_identity`] / [`inverse_identity`], AV1 §7.13.2.11–.15).
//!
//! The 2-D assembly that selects per `PlaneTxType` and applies the per-pass normalization shifts
//! (AV1 §7.13.3) is tracked in `gamut-avif/STATUS.md`.
//!
//! Alongside the transforms, [`mod@math`] exposes the small AV1 §4.7 integer arithmetic
//! primitives ([`round2`], [`round2_signed`], [`clip3`]) and the shared forward-quantize
//! rounding ([`round_div_nearest`]) that the codec crates build on, and [`mod@mulaw`] adds
//! µ-law companding/quantization ([`mu_compress`] / [`mu_expand`] / [`mu_quantize`] /
//! [`mu_dequantize`]).
#![forbid(unsafe_code)]

mod adst;
mod butterfly;
mod dct;
mod identity;
mod math;
mod mulaw;
mod wht;

pub use adst::{flip_in_place, forward_adst, inverse_adst};
pub use dct::{forward_dct, inverse_dct};
pub use identity::{forward_identity, inverse_identity};
pub use math::{clip3, round_div_nearest, round2, round2_signed};
pub use mulaw::{mu_compress, mu_dequantize, mu_expand, mu_quantize};
pub use wht::{fwht4x4, iwht4x4};
