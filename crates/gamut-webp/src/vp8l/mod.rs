//! VP8L — the WebP lossless still-image bitstream (RFC 9649 §3).
//!
//! The lossless format codes 8-bit ARGB pixels with canonical prefix (Huffman) codes over a small
//! set of reversible transforms ([`transform`], §3.5), LZ77 backward references ([`lz77`], §3.6),
//! and a color cache ([`color_cache`], §3.6.3), all read/written with an LSB-first bit stream
//! ([`bit_io`], §3.3). The component breakdown and milestones live in `../STATUS.md` (sections B-F).
//!
//! These modules currently expose the declarative type surface the encoder and decoder fill in. The
//! M0 path — [`header`] + subtract-green ([`transform`]) + [`prefix`] codes + literal pixels — is
//! under construction; the `encode`/`decode` entry points land with it.

pub mod bit_io;
pub mod color_cache;
pub mod header;
pub mod lz77;
pub mod prefix;
pub mod transform;

pub use bit_io::{BitReader, BitWriter};
pub use header::{VP8L_SIGNATURE, Vp8lHeader};
pub use transform::Vp8lTransform;
