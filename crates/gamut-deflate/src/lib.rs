//! `gamut-deflate` — a pure-Rust **DEFLATE** (RFC 1951) and **zlib** (RFC 1950) *encoder*.
//!
//! DEFLATE is the compression engine behind PNG's `IDAT`/`zTXt`/`iCCP`, TIFF's `Compression=8`,
//! gzip, and zlib streams generally. This crate provides it as a shared primitive, sitting below the
//! codec crates alongside [`gamut-bitstream`](https://docs.rs/gamut-bitstream) in the workspace's
//! dependency graph, with no internal dependencies of its own.
//!
//! # Encoder only
//!
//! Following gamut's encoder-first philosophy, this crate **does not decode** — inflating DEFLATE is
//! a thoroughly solved problem with strong implementations everywhere, so there is nothing to gain
//! by reimplementing it. Correctness is instead proven differentially: the dev-only `zlib-oracle`
//! inflates the encoder's output with the canonical C `zlib` and asserts it round-trips to the
//! original bytes.
//!
//! # Space efficiency
//!
//! The encoder is tuned for size over speed (see [`Level`]). [`Level::Best`] applies a
//! zopfli-style optimal parse and package-merge length-limited Huffman codes to approach the
//! smallest streams achievable in the DEFLATE format; lower levels trade ratio for speed.
//!
//! # Example
//!
//! ```
//! use gamut_deflate::{DeflateEncoder, Level};
//!
//! let data = b"the quick brown fox jumps over the lazy dog".repeat(8);
//! let mut zlib_stream = Vec::new();
//! let written = DeflateEncoder::new()
//!     .with_level(Level::Best)
//!     .zlib_compress(&data, &mut zlib_stream);
//! assert_eq!(written, zlib_stream.len());
//! // A zlib stream starts with the 0x78 CMF byte.
//! assert_eq!(zlib_stream[0], 0x78);
//! ```
#![forbid(unsafe_code)]

mod adler32;
mod bitwriter;
mod block;
mod encoder;
mod zlib;

pub use adler32::adler32;
pub use encoder::{DeflateEncoder, Level};
