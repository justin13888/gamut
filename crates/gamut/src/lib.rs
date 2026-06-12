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
//!
//! The `primitives` feature additionally re-exports the shared building blocks the codecs are
//! made of — `color` (pixel formats / CICP), `dsp` (transforms), and `bitstream` (bit writer
//! + AV1 symbol coder) — for tooling and sandbox use such as the `gamut` CLI.
//!
//! The `metadata` feature re-exports the shared image-metadata primitives (issue #34) — the
//! TIFF/IFD container core (`ifd`) plus, as they land, the per-format crates (`exif`/`xmp`/`icc`/
//! `iptc`) and the `metadata` facade that unifies them — for tooling and for the container crates
//! to consume.
#![forbid(unsafe_code)]

pub use gamut_core as core;

#[cfg(feature = "metadata")]
pub use gamut_exif as exif;
#[cfg(feature = "metadata")]
pub use gamut_icc as icc;
#[cfg(feature = "metadata")]
pub use gamut_ifd as ifd;
#[cfg(feature = "metadata")]
pub use gamut_iptc as iptc;
#[cfg(feature = "metadata")]
pub use gamut_metadata as metadata;
#[cfg(feature = "metadata")]
pub use gamut_xmp as xmp;

#[cfg(feature = "primitives")]
pub use gamut_bitstream as bitstream;
#[cfg(feature = "primitives")]
pub use gamut_color as color;
#[cfg(feature = "primitives")]
pub use gamut_dsp as dsp;

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
#[cfg(feature = "tonemap")]
pub use gamut_tonemap as tonemap;
#[cfg(feature = "vvc")]
pub use gamut_vvc as vvc;
#[cfg(feature = "webp")]
pub use gamut_webp as webp;
