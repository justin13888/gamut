//! Tone-mapping math primitives for the gamut image-encoding workspace.
//!
//! Tone mapping compresses a high-dynamic-range (HDR) signal into a range a target display or
//! encoder can represent, trading absolute luminance for preserved relative contrast. This crate
//! provides the *math primitives* to do that — a small set of built-in tone curves plus an
//! extensible [`ToneCurve`] trait so consumers can supply their own — and deliberately stops
//! there: it performs no colour-space conversion, gamut mapping, or pixel I/O.
//!
//! # Modules
//!
//! - [`curve`] — the [`ToneCurve`] trait every operator implements, with an in-place slice helper
//!   and a blanket impl so a plain `Fn(f32) -> f32` closure is itself a curve.
//! - [`operators`] — the built-in curves: [`Reinhard`](operators::Reinhard) and
//!   [`ReinhardExtended`](operators::ReinhardExtended), plus the [`Linear`](operators::Linear)
//!   passthrough and [`Clamp`](operators::Clamp).
//! - [`constants`] — per-operator parameter defaults. (Absolute reference luminances in nits live
//!   in [`gamut_core::luminance`].)
//!
//! # Inputs and the HDR-to-SDR pipeline
//!
//! Curves take a single non-negative *linear-light* value — a per-channel RGB component or a
//! luminance. A full HDR-to-SDR conversion is three steps, of which this crate owns the middle one:
//!
//! 1. Linearize the source signal, identified by its transfer function (e.g. `Pq` or `Hlg` from
//!    `gamut_color`'s `TransferCharacteristics`).
//! 2. Apply a [`ToneCurve`] from this crate to the linear values.
//! 3. Re-encode through the target SDR transfer function (e.g. `Srgb`).
//!
//! Keeping the boundary here means a curve is just `f32 -> f32` and reusable outside any colour
//! pipeline.
//!
//! # Example
//!
//! ```
//! use gamut_tonemap::{ToneCurve, operators::ReinhardExtended};
//!
//! let curve = ReinhardExtended::new(4.0)?; // linear white point = 4.0
//! let display = curve.map(2.5);
//! assert!(display > 0.0 && display < 1.0);
//!
//! // Any closure is also a curve:
//! let gamma = |x: f32| x.powf(1.0 / 2.2);
//! let mut linear = [0.2_f32, 0.8, 3.0];
//! gamma.map_slice(&mut linear);
//! # Ok::<(), gamut_core::Error>(())
//! ```
#![forbid(unsafe_code)]

pub mod constants;
pub mod curve;
pub mod operators;

#[doc(inline)]
pub use curve::ToneCurve;
