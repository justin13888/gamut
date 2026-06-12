//! TIFF compression schemes, selected by the `Compression` tag (259).
//!
//! Baseline TIFF readers must handle the uncompressed, Modified Huffman, and PackBits schemes;
//! the remainder are extensions (TIFF 6.0 Part 2). Each scheme is decoded/encoded per strip or
//! tile; the per-scheme codecs land in later phases.

pub mod ccitt;
pub mod lzw;
pub mod packbits;
pub mod predictor;

/// A compression scheme applied to a strip or tile of image data.
///
/// The discriminants are documented with their on-disk `Compression` tag values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Compression {
    /// `1` — no compression; samples are packed into bytes as tightly as possible.
    #[default]
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

impl Compression {
    /// Returns the scheme for an on-disk `Compression` tag value, or `None` if unrecognised.
    #[must_use]
    pub fn from_code(code: u32) -> Option<Self> {
        Some(match code {
            1 => Compression::None,
            2 => Compression::CcittRle,
            3 => Compression::CcittGroup3Fax,
            4 => Compression::CcittGroup4Fax,
            5 => Compression::Lzw,
            6 => Compression::OldJpeg,
            7 => Compression::Jpeg,
            8 | 32946 => Compression::Deflate,
            32773 => Compression::PackBits,
            _ => return None,
        })
    }

    /// Returns the on-disk `Compression` tag value.
    #[must_use]
    pub fn code(self) -> u16 {
        match self {
            Compression::None => 1,
            Compression::CcittRle => 2,
            Compression::CcittGroup3Fax => 3,
            Compression::CcittGroup4Fax => 4,
            Compression::Lzw => 5,
            Compression::OldJpeg => 6,
            Compression::Jpeg => 7,
            Compression::Deflate => 8,
            Compression::PackBits => 32773,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compression_codes_round_trip() {
        for c in [
            Compression::None,
            Compression::CcittRle,
            Compression::CcittGroup3Fax,
            Compression::CcittGroup4Fax,
            Compression::Lzw,
            Compression::OldJpeg,
            Compression::Jpeg,
            Compression::Deflate,
            Compression::PackBits,
        ] {
            assert_eq!(Compression::from_code(u32::from(c.code())), Some(c));
        }
        assert_eq!(Compression::from_code(32946), Some(Compression::Deflate));
        assert_eq!(Compression::from_code(99), None);
    }
}
