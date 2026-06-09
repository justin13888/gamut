//! Shared digital signal processing routines for the gamut codecs.
//!
//! Provides the AV1 transform kernels:
//! - the lossless 4×4 Walsh–Hadamard pair ([`fwht4x4`] / [`iwht4x4`], AV1 §7.13.2.10), and
//! - the discrete cosine transform pair ([`forward_dct`] / [`inverse_dct`], AV1 §7.13.2.2–.3)
//!   used by *lossy* AV1 coding.
//!
//! The asymmetric discrete sine transforms (AV1 §7.13.2.4–.9) and the 2-D assembly that applies
//! the per-pass normalization shifts (AV1 §7.13.3) are tracked in `gamut-avif/STATUS.md`.
#![forbid(unsafe_code)]

mod butterfly;
mod dct;
mod wht;

pub use dct::{forward_dct, inverse_dct};
pub use wht::{fwht4x4, iwht4x4};
