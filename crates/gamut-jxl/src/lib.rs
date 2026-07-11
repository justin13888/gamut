//! `gamut-jxl` — a JPEG XL (JXL) image **encoder and decoder**.
//!
//! Unlike the clean-slate codecs elsewhere in the workspace, gamut-jxl is a thin, safe layer over
//! the JPEG XL reference implementations:
//!
//! - the **encoder** wraps the ISO/IEC 18181 reference implementation, libjxl 0.12.0, statically
//!   linked through [`gamut-jxl-sys`](https://crates.io/crates/gamut-jxl-sys) — the Rust ecosystem
//!   has no reference-quality JPEG XL *encoder*, so wrapping libjxl is the sole spec-faithful path;
//! - the **decoder** wraps the pure-Rust [`jxl`](https://crates.io/crates/jxl) crate (jxl-rs, the
//!   libjxl org's Rust decoder), so decoding needs no C toolchain and works on `wasm32`.
//!
//! The departure from the workspace's "pure Rust" norm — and its licensing and build implications —
//! is a maintainer-confirmed decision recorded in the crate `README.md`.
//!
//! # Encoding
//!
//! Build a [`JxlEncoder`] — [`JxlEncoder::lossless`] (the default) or [`JxlEncoder::lossy`] with a
//! validated [`Distance`] — tune it with [`JxlEncoder::with_effort`] and
//! [`JxlEncoder::with_container`], then encode any of the eight supported pixel layouts (8/16-bit
//! grayscale, gray+alpha, RGB, RGBA) through the [`EncodeImage`](gamut_core::EncodeImage) trait.
//! Requires the `encode` feature (on by default; a no-op on `wasm32`).
//!
//! # Decoding
//!
//! Build a [`JxlDecoder`] and decode a JPEG XL byte stream — bare codestream or ISO BMFF container —
//! into any of the same eight pixel layouts through the [`DecodeImage`](gamut_core::DecodeImage)
//! trait. The decoder converts internally where the request and the stream differ (grayscale →
//! RGB, opaque-alpha padding, alpha dropping) and refuses lossy guesses such as reading a colour
//! image back as grayscale. Requires the `decode` feature (on by default; available everywhere,
//! `wasm32` included).
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
//! drives libjxl (hence `deny` rather than `forbid`). The decoder is 100% safe Rust. Because the
//! native libjxl build is unavailable on `wasm32`, the encoder is compiled in only for
//! `all(feature = "encode", not(target_arch = "wasm32"))`; the decoder has no such restriction, so
//! on `wasm32` the crate builds as a decode-only codec.
#![deny(unsafe_code)]

#[cfg(all(feature = "encode", not(target_arch = "wasm32")))]
mod config;
#[cfg(feature = "decode")]
mod convert;
#[cfg(feature = "decode")]
mod decoder;
#[cfg(all(feature = "encode", not(target_arch = "wasm32")))]
mod encoder;
// The error-mapping module carries an encoder half (libjxl statuses) and a decoder half (jxl-rs
// errors), each independently feature-gated inside the module; it is present whenever either codec
// half is compiled.
#[cfg(any(
    all(feature = "encode", not(target_arch = "wasm32")),
    feature = "decode"
))]
mod error;
#[cfg(all(feature = "encode", not(target_arch = "wasm32")))]
mod ffi;

#[cfg(all(feature = "encode", not(target_arch = "wasm32")))]
pub use config::{Container, Distance, Effort};
#[cfg(feature = "decode")]
pub use decoder::JxlDecoder;
#[cfg(all(feature = "encode", not(target_arch = "wasm32")))]
pub use encoder::JxlEncoder;
