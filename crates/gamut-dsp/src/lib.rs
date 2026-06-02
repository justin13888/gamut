//! Shared digital signal processing routines for the gamut codecs.
//!
//! Currently provides the AV1 lossless 4×4 transform pair ([`fwht4x4`] / [`iwht4x4`]). The
//! discrete cosine / asymmetric discrete sine transforms used by *lossy* AV1 coding (AV1
//! §7.13.2.2–.9) are deferred to milestone M1 (see `gamut-avif/STATUS.md`).
#![forbid(unsafe_code)]

mod wht;

pub use wht::{fwht4x4, iwht4x4};
