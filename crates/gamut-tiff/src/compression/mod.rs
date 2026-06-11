//! TIFF compression schemes, selected by the `Compression` tag (259).
//!
//! Baseline TIFF readers must handle the uncompressed, Modified Huffman, and PackBits schemes;
//! the remainder are extensions (TIFF 6.0 Part 2). Each scheme is decoded/encoded per strip or
//! tile; the per-scheme codecs land in later phases.

/// A compression scheme applied to a strip or tile of image data.
///
/// The discriminants match the on-disk `Compression` tag values.
pub enum Compression {
    /// `1` — no compression; samples are packed into bytes as tightly as possible.
    None,
    /// `2` — CCITT Group 3 1-Dimensional Modified Huffman run-length encoding (TIFF 6.0 §10).
    CcittRle,
    /// `3` — CCITT T.4 (Group 3) bilevel fax encoding (TIFF 6.0 §11).
    CcittGroup3Fax,
    /// `4` — CCITT T.6 (Group 4) bilevel fax encoding (TIFF 6.0 §11).
    CcittGroup4Fax,
    /// `5` — LZW (TIFF 6.0 §13).
    Lzw,
    /// `6` — the deprecated old-style JPEG process (TIFF 6.0 §22).
    OldJpeg,
    /// `7` — JPEG (the redefined "new-style" process; TIFF Technical Note 2).
    Jpeg,
    /// `8` — Deflate/zlib (Adobe). Post-6.0 extension; out of the current campaign's scope.
    Deflate,
    /// `32773` — PackBits, a simple byte-oriented run-length scheme (TIFF 6.0 §9).
    PackBits,
}
