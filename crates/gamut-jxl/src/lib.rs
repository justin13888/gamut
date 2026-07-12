//! `gamut-jxl` — a JPEG XL (JXL) image **encoder and decoder**.
//!
//! Unlike the clean-slate codecs elsewhere in the workspace, gamut-jxl is a thin, safe layer over
//! the JPEG XL reference implementations:
//!
//! - the **encoder** wraps the ISO/IEC 18181 reference implementation, libjxl 0.12.0, statically
//!   linked through [`gamut-jxl-sys`](https://crates.io/crates/gamut-jxl-sys) — the Rust ecosystem
//!   has no reference-quality JPEG XL *encoder*, so wrapping libjxl is the sole spec-faithful path;
//! - the **decoder** wraps the pure-Rust [`jxl`](https://crates.io/crates/jxl) crate (jxl-rs, the
//!   libjxl org's Rust decoder), so decoding needs no C toolchain and works on every `wasm32`
//!   target.
//!
//! The departure from the workspace's "pure Rust" norm — and its licensing and build implications —
//! is a maintainer-confirmed decision recorded in the crate `README.md`.
//!
//! # Encoding
//!
//! Build a [`JxlEncoder`] — [`JxlEncoder::lossless`] (the default) or [`JxlEncoder::lossy`] with a
//! validated [`Distance`] — tune it with the chainable builders ([`JxlEncoder::with_effort`],
//! [`JxlEncoder::with_container`], [`JxlEncoder::with_color`] for sRGB/linear/PQ/HLG/ICC
//! signalling, [`JxlEncoder::with_orientation`], and [`JxlEncoder::with_exif`] /
//! [`JxlEncoder::with_xmp`] for container metadata boxes), then encode any of the eight supported
//! pixel layouts (8/16-bit grayscale, gray+alpha, RGB, RGBA) through the
//! [`EncodeImage`](gamut_core::EncodeImage) trait. [`JxlEncoder::recompress_jpeg`] losslessly
//! re-packs an existing JPEG so the original is reconstructible bit-for-bit (jbrd).
//! Requires the `encode` feature (on by default; a no-op on `wasm32` targets other than
//! emscripten — see below).
//!
//! # Decoding
//!
//! Build a [`JxlDecoder`] and decode a JPEG XL byte stream — bare codestream or ISO BMFF container —
//! into any of the same eight pixel layouts through the [`DecodeImage`](gamut_core::DecodeImage)
//! trait. The decoder converts internally where the request and the stream differ (grayscale →
//! RGB, opaque-alpha padding, alpha dropping) and refuses lossy guesses such as reading a colour
//! image back as grayscale. Requires the `decode` feature (on by default; available everywhere,
//! every `wasm32` target included).
//!
//! # Example: lossless round-trip
//!
//! With the default features (`encode` + `decode`), a lossless stream decodes back bit-exact. The
//! example needs both codec halves, so it is compiled and run only where they are present — the
//! `cargo test --doc` environment on a non-`wasm32` host:
//!
//! ```
//! use gamut_core::{DecodeImage, Dimensions, EncodeImage, ImageBuf, ImageRef, Rgb8};
//! use gamut_jxl::{JxlDecoder, JxlEncoder};
//!
//! // A 4×4 8-bit RGB test image.
//! let dims = Dimensions { width: 4, height: 4 };
//! let pixels: Vec<u8> = (0..4 * 4 * 3).map(|i| i as u8).collect();
//! let image = ImageRef::<Rgb8>::new(&pixels, dims)?;
//!
//! // Lossless encode, then decode straight back to the same layout.
//! let stream = JxlEncoder::lossless().encode_to_vec(image)?;
//! let decoded: ImageBuf<Rgb8> = JxlDecoder::new().decode_image(&stream)?;
//!
//! // Lossless output is bit-exact to the input.
//! assert_eq!(decoded.dimensions(), dims);
//! assert_eq!(decoded.as_samples(), pixels.as_slice());
//! # Ok::<(), gamut_core::Error>(())
//! ```
//!
//! # Safety and portability
//!
//! The crate is `#![deny(unsafe_code)]`; all `unsafe` is confined to the single `ffi` module that
//! drives libjxl (hence `deny` rather than `forbid`). The decoder is 100% safe Rust and available
//! on every target.
//!
//! The encoder is compiled in for
//! `all(feature = "encode", any(not(target_arch = "wasm32"), target_os = "emscripten"))` — that
//! is, everywhere except `wasm32` targets that emscripten does not cover:
//!
//! - **`wasm32-unknown-emscripten`** gets the full encoder: libjxl officially supports wasm via
//!   emscripten, and `gamut-jxl-sys` builds it with the emsdk toolchain (`emcc` on `PATH`).
//! - **`wasm32-unknown-unknown`** (the wasm-bindgen/browser target) is decode-only, permanently by
//!   toolchain boundary rather than by workaround: no C/C++ compiler emits archives for that ABI,
//!   so no build configuration could link libjxl there. A pure-Rust JPEG XL encoder is the only
//!   thing that could ever change this (jxl-rs ships none).
#![deny(unsafe_code)]

#[cfg(all(
    feature = "encode",
    any(not(target_arch = "wasm32"), target_os = "emscripten")
))]
mod config;
#[cfg(feature = "decode")]
mod convert;
#[cfg(feature = "decode")]
mod decoder;
#[cfg(all(
    feature = "encode",
    any(not(target_arch = "wasm32"), target_os = "emscripten")
))]
mod encoder;
// The error-mapping module carries an encoder half (libjxl statuses) and a decoder half (jxl-rs
// errors), each independently feature-gated inside the module; it is present whenever either codec
// half is compiled.
#[cfg(any(
    all(
        feature = "encode",
        any(not(target_arch = "wasm32"), target_os = "emscripten")
    ),
    feature = "decode"
))]
mod error;
#[cfg(all(
    feature = "encode",
    any(not(target_arch = "wasm32"), target_os = "emscripten")
))]
mod ffi;

#[cfg(all(
    feature = "encode",
    any(not(target_arch = "wasm32"), target_os = "emscripten")
))]
pub use config::{ColorSpec, Container, Distance, Effort, Orientation};
#[cfg(feature = "decode")]
pub use decoder::JxlDecoder;
#[cfg(all(
    feature = "encode",
    any(not(target_arch = "wasm32"), target_os = "emscripten")
))]
pub use encoder::JxlEncoder;
