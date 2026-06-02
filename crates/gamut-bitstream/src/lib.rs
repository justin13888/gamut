//! Low-level bit writers and entropy coders shared by the gamut codecs.
//!
//! The pieces here are the encoder-side mirror of the parsing processes defined in the AV1
//! Bitstream & Decoding Process Specification (`references/av1/av1-spec.pdf`):
//!
//! - [`BitWriter`] — most-significant-bit-first fixed-width fields (`f(n)`) and byte alignment,
//!   used by the AV1 uncompressed sequence/frame headers (AV1 §4, §8.1).
//! - [`write_leb128`] / [`leb128_len`] — unsigned LEB128 used for OBU sizes (AV1 §4.10.5, Annex B).
//! - [`SymbolEncoder`] — the AV1 multi-symbol arithmetic (range) coder, derived by inverting the
//!   symbol *decoder* of AV1 §8.2. It is the entropy back-end for coded tile data.
//!
//! The forward-looking ANS / Huffman coders named in the workspace plan (for AV2 / JPEG XL) are
//! not implemented yet; they will join this crate behind their own modules.
#![forbid(unsafe_code)]

mod bitwriter;
mod leb128;
mod symbol;

pub use bitwriter::BitWriter;
pub use leb128::{leb128_len, write_leb128};
pub use symbol::SymbolEncoder;
