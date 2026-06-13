//! The public DEFLATE encoder: a [`DeflateEncoder`] builder with a [`Level`] knob and the
//! `compress` (raw RFC 1951) / `zlib_compress` (RFC 1950) entry points.

use crate::adler32::adler32;
use crate::{block, lz77, zlib};

/// Compression effort, trading encode time for output size. Every level produces a correct stream;
/// they differ only in ratio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Level {
    /// Stored (uncompressed) blocks only — the always-correct floor and an upper bound on size.
    Store,
    /// Fast: greedy matching with fixed Huffman codes.
    Fast,
    /// Balanced default: lazy matching with per-block dynamic Huffman codes.
    #[default]
    Default,
    /// Smallest output: a zopfli-style optimal parse with package-merge length-limited codes.
    /// Slowest; intended for write-once assets where size dominates.
    Best,
}

/// A reusable DEFLATE / zlib encoder configured by a [`Level`].
#[derive(Debug, Clone)]
pub struct DeflateEncoder {
    level: Level,
}

impl Default for DeflateEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl DeflateEncoder {
    /// Creates an encoder at [`Level::Default`].
    #[must_use]
    pub fn new() -> Self {
        Self {
            level: Level::Default,
        }
    }

    /// Sets the compression [`Level`].
    #[must_use]
    pub fn with_level(mut self, level: Level) -> Self {
        self.level = level;
        self
    }

    /// Encodes `data` as a raw DEFLATE stream (RFC 1951), appending to `out` and returning the
    /// number of bytes written. Any input — including empty — produces a valid stream.
    pub fn compress(&self, data: &[u8], out: &mut Vec<u8>) -> usize {
        let body = self.deflate_body(data);
        out.extend_from_slice(&body);
        body.len()
    }

    /// Encodes `data` as a zlib stream (RFC 1950): a 2-byte header, the DEFLATE body, and a
    /// big-endian Adler-32 trailer. Appends to `out` and returns the number of bytes written. This
    /// is the stream PNG's `IDAT` carries.
    pub fn zlib_compress(&self, data: &[u8], out: &mut Vec<u8>) -> usize {
        let start = out.len();
        out.extend_from_slice(&zlib::header(self.level));
        out.extend_from_slice(&self.deflate_body(data));
        out.extend_from_slice(&adler32(1, data).to_be_bytes());
        out.len() - start
    }

    /// Builds the DEFLATE body for `data`, choosing the smallest block encoding the level offers.
    fn deflate_body(&self, data: &[u8]) -> Vec<u8> {
        match self.level {
            // The uncompressed floor.
            Level::Store => block::stored(data),
            // LZ77 parse, then keep the smaller of stored vs fixed-Huffman. Dynamic Huffman and a
            // per-block choice land in later phases.
            Level::Fast | Level::Default | Level::Best => {
                let tokens = lz77::parse(data, self.max_chain());
                let stored = block::stored(data);
                let fixed = block::fixed(&tokens);
                if fixed.len() <= stored.len() {
                    fixed
                } else {
                    stored
                }
            }
        }
    }

    /// LZ77 match-finder search depth for this level — the time/ratio knob.
    fn max_chain(&self) -> usize {
        match self.level {
            Level::Store => 0,
            Level::Fast => 16,
            Level::Default => 128,
            Level::Best => 1024,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_appends_and_reports_length() {
        let mut out = vec![0xEE];
        let n = DeflateEncoder::new().compress(b"abc", &mut out);
        assert_eq!(n, out.len() - 1);
        assert_eq!(out[0], 0xEE, "existing bytes are preserved");
    }

    #[test]
    fn zlib_compress_has_header_and_trailer() {
        let data = b"abcabcabc";
        let mut out = Vec::new();
        let n = DeflateEncoder::new().zlib_compress(data, &mut out);
        assert_eq!(n, out.len());
        assert_eq!(out[0], 0x78, "zlib CMF byte");
        // Trailer is the big-endian Adler-32 of the *uncompressed* data.
        let trailer = &out[out.len() - 4..];
        assert_eq!(trailer, adler32(1, data).to_be_bytes());
    }

    #[test]
    fn fixed_huffman_beats_stored_on_ascii_text() {
        // All-ASCII bytes (< 144) get 8-bit fixed codes, so a fixed block undercuts stored's
        // per-block byte overhead. The encoder must pick it.
        let data = b"the quick brown fox jumps over the lazy dog. ".repeat(40);
        let mut fixed = Vec::new();
        DeflateEncoder::new()
            .with_level(Level::Fast)
            .zlib_compress(&data, &mut fixed);
        let mut store = Vec::new();
        DeflateEncoder::new()
            .with_level(Level::Store)
            .zlib_compress(&data, &mut store);
        assert!(
            fixed.len() < store.len(),
            "fixed {} should beat stored {}",
            fixed.len(),
            store.len()
        );
    }
}
