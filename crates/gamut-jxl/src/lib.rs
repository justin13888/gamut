//! `gamut-jxl` — a JPEG XL (JXL) image **encoder**, wrapping the reference libjxl 0.12.0.
//!
//! Unlike the clean-slate codecs elsewhere in the workspace, gamut-jxl's encoder is a thin, safe
//! layer over the ISO/IEC 18181 reference implementation, libjxl, statically linked through
//! [`gamut-jxl-sys`](https://crates.io/crates/gamut-jxl-sys). The Rust ecosystem has no
//! reference-quality JPEG XL *encoder* (the pure-Rust `jxl` crate decodes only), so wrapping libjxl
//! is the sole path to a spec-faithful encoder; the departure from the workspace's "pure Rust"
//! norm — and its licensing and build implications — is a maintainer-confirmed decision recorded in
//! the crate `README.md`.
//!
//! # Encoding
//!
//! Build a [`JxlEncoder`] — [`JxlEncoder::lossless`] (the default) or [`JxlEncoder::lossy`] with a
//! validated [`Distance`] — tune it with [`JxlEncoder::with_effort`] and
//! [`JxlEncoder::with_container`], then encode any of the eight supported pixel layouts (8/16-bit
//! grayscale, gray+alpha, RGB, RGBA) through the [`EncodeImage`](gamut_core::EncodeImage) trait.
//!
//! # Decoding
//!
//! A pure-Rust decoder wrapping the `jxl` crate arrives in a following change (behind a `decode`
//! feature); this version is encoder-only.
//!
//! # Safety and portability
//!
//! The crate is `#![deny(unsafe_code)]`; all `unsafe` is confined to the single [`ffi`] module that
//! drives libjxl (hence `deny` rather than `forbid`). Because the native libjxl build is unavailable
//! on `wasm32`, the encoder is compiled in only for `all(feature = "encode", not(target_arch =
//! "wasm32"))`; on wasm32 the crate builds with no encoder (making room for the future wasm-friendly
//! pure-Rust decoder).
#![deny(unsafe_code)]

#[cfg(all(feature = "encode", not(target_arch = "wasm32")))]
mod config;
#[cfg(all(feature = "encode", not(target_arch = "wasm32")))]
mod encoder;
#[cfg(all(feature = "encode", not(target_arch = "wasm32")))]
mod error;
#[cfg(all(feature = "encode", not(target_arch = "wasm32")))]
mod ffi;

#[cfg(all(feature = "encode", not(target_arch = "wasm32")))]
pub use config::{Container, Distance, Effort};
#[cfg(all(feature = "encode", not(target_arch = "wasm32")))]
pub use encoder::JxlEncoder;
