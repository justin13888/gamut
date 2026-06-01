//! `gamut` — the umbrella crate for a collection of space-efficient image encoding libraries.
//!
//! This crate re-exports the high-level APIs of the format-specific crates behind Cargo
//! features, so a consumer compiles only the codecs they need:
//!
//! ```toml
//! gamut = { version = "0.1", features = ["avif", "jxl"] }
//! ```
//!
//! [`core`] (re-exported from [`gamut-core`](https://crates.io/crates/gamut-core)) is always
//! available and provides the shared [`core::Encoder`] / [`core::Decoder`] traits and the
//! common [`core::Error`] type. Each format module appears only when its feature is enabled.
#![forbid(unsafe_code)]

pub use gamut_core as core;

#[cfg(feature = "av1")]
pub use gamut_av1 as av1;
#[cfg(feature = "av2")]
pub use gamut_av2 as av2;
#[cfg(feature = "avif")]
pub use gamut_avif as avif;
#[cfg(feature = "heic")]
pub use gamut_heic as heic;
#[cfg(feature = "jxl")]
pub use gamut_jxl as jxl;
#[cfg(feature = "vvc")]
pub use gamut_vvc as vvc;
#[cfg(feature = "webp")]
pub use gamut_webp as webp;
