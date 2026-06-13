//! The public DEFLATE encoder: a [`DeflateEncoder`] builder with a [`Level`] knob and the
//! `compress` (raw RFC 1951) / `zlib_compress` (RFC 1950) entry points.

use crate::adler32::adler32;
use crate::bitwriter::BitWriter;
use crate::{block, zlib};

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
        let start = out.len();
        let mut w = BitWriter::new();
        self.deflate(&mut w, data);
        out.extend_from_slice(&w.finish());
        out.len() - start
    }

    /// Encodes `data` as a zlib stream (RFC 1950): a 2-byte header, the DEFLATE body, and a
    /// big-endian Adler-32 trailer. Appends to `out` and returns the number of bytes written. This
    /// is the stream PNG's `IDAT` carries.
    pub fn zlib_compress(&self, data: &[u8], out: &mut Vec<u8>) -> usize {
        let start = out.len();
        out.extend_from_slice(&zlib::header(self.level));
        let mut w = BitWriter::new();
        self.deflate(&mut w, data);
        out.extend_from_slice(&w.finish());
        out.extend_from_slice(&adler32(1, data).to_be_bytes());
        out.len() - start
    }

    /// Emits the DEFLATE body for `data` into `w`.
    fn deflate(&self, w: &mut BitWriter, data: &[u8]) {
        // D1: stored blocks for every level. Later phases route Fast/Default/Best through the
        // fixed- and dynamic-Huffman block coders for actual compression.
        let _ = self.level;
        block::write_stored(w, data);
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
}
