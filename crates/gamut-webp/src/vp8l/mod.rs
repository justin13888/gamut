//! VP8L — the WebP lossless still-image bitstream (RFC 9649 §3).
//!
//! The lossless format codes 8-bit ARGB pixels with canonical prefix (Huffman) codes over a small
//! set of reversible transforms ([`transform`], §3.5), LZ77 backward references ([`lz77`], §3.6),
//! and a color cache ([`color_cache`], §3.6.3), all read/written with an LSB-first bit stream
//! ([`bit_io`], §3.3). The component breakdown and milestones live in `../STATUS.md` (sections B-F).
//!
//! The full lossless path is implemented: [`decoder::decode`] reads any conformant VP8L stream and
//! [`encoder::encode`] emits a bit-exact-lossless one. Each module cites the spec section it covers;
//! the component breakdown is tracked in `../STATUS.md`.

pub mod bit_io;
pub mod color_cache;
pub mod decoder;
pub mod encoder;
pub mod header;
pub mod lz77;
pub mod prefix;
pub mod transform;

pub use bit_io::{BitReader, BitWriter};
pub use color_cache::ColorCache;
pub use header::{VP8L_SIGNATURE, Vp8lHeader};
pub use transform::Vp8lTransform;

/// Ceiling division `⌈num / den⌉`, used to size sub-resolution images and bundled widths
/// (the spec's `DIV_ROUND_UP`). Returns 0 when `den` is 0. All VP8L callers pass `num <= 16384`
/// and `den` a power of two, so the `num + den - 1` cannot overflow.
#[must_use]
pub(crate) const fn div_round_up(num: u32, den: u32) -> u32 {
    if den == 0 {
        return 0;
    }
    num.div_ceil(den)
}
