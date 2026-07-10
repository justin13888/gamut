//! AV1 transform kernels (AV1 §7.13.2).
//!
//! The 1-D kernels the AV1-family codecs assemble into 2-D transforms:
//! - the discrete cosine pair ([`forward_dct`] / [`inverse_dct`], §7.13.2.2–.3),
//! - the asymmetric discrete sine pair ([`forward_adst`] / [`inverse_adst`], §7.13.2.4–.9 —
//!   DST-VII at size 4, DST-IV at 8/16),
//! - the identity transforms ([`forward_identity`] / [`inverse_identity`], §7.13.2.11–.15), and
//! - the complete lossless 4×4 Walsh–Hadamard block pair ([`forward_wht4x4`] /
//!   [`inverse_wht4x4`], §7.13.2.10).
//!
//! This module is the kernel library; the `gamut-av1` *crate* is the codec that drives it: the
//! 2-D row/column assembly — per-`TxType` kernel selection, the per-pass normalization shifts,
//! and the FLIPADST sample-order flips (§7.13.3) — lives there.
//!
//! The `inverse_*` kernels are normative decoder processes and must be bit-exact; the
//! `forward_*` kernels are encoder choices, guaranteed consistent with their paired inverse and
//! reconciled in absolute scale by the 2-D assembly's shifts.
//!
//! The 1-D kernels operate in place on `&mut [i64]` (the intermediate headroom the 2-D passes
//! need); the WHT pair is a complete 4×4 block transform over by-value `[i32; 16]` arrays,
//! whose exact-roundtrip domain provably fits `i32`.

mod adst;
mod butterfly;
mod dct;
mod identity;
mod wht;

pub use adst::{forward_adst, inverse_adst};
pub use dct::{forward_dct, inverse_dct};
pub use identity::{forward_identity, inverse_identity};
pub use wht::{forward_wht4x4, inverse_wht4x4};
