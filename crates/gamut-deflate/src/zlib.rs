//! zlib stream framing (RFC 1950 §2.2): the 2-byte header that wraps a raw DEFLATE stream.
//!
//! A full zlib stream is `header || deflate || adler32`. This module builds the header; the encoder
//! appends the DEFLATE body and the big-endian Adler-32 trailer.

use crate::encoder::Level;

/// The 2-byte zlib header for the given compression level.
///
/// `CMF = 0x78` encodes `CM = 8` (DEFLATE) and `CINFO = 7` (a 32 KiB window). `FLG` carries
/// `FDICT = 0`, a `FLEVEL` hint reflecting `level`, and a 5-bit `FCHECK` chosen so the 16-bit value
/// `(CMF << 8) | FLG` is a multiple of 31 (RFC 1950 §2.2). These are the familiar `78 01` / `78 9C` /
/// `78 DA` headers.
pub(crate) fn header(level: Level) -> [u8; 2] {
    const CMF: u16 = 0x78; // CM = 8 (deflate), CINFO = 7 (32K window)
    let flevel: u16 = match level {
        // 0 = fastest algorithm, 1 = fast, 2 = default, 3 = maximum/slowest. Purely advisory.
        Level::Store | Level::Fast => 0,
        Level::Default => 2,
        Level::Best => 3,
    };
    let flg_base = flevel << 6; // FLEVEL in bits 6-7; FDICT (bit 5) and FCHECK (bits 0-4) start at 0.
    let mut value = (CMF << 8) | flg_base;
    let rem = value % 31;
    if rem != 0 {
        // Setting the low FCHECK bits to (31 - rem) makes the header divisible by 31. The addition
        // never carries past bit 4 because 31 - rem <= 31 and FCHECK started at 0.
        value += 31 - rem;
    }
    value.to_be_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_canonical_headers() {
        assert_eq!(header(Level::Fast), [0x78, 0x01]);
        assert_eq!(header(Level::Store), [0x78, 0x01]);
        assert_eq!(header(Level::Default), [0x78, 0x9C]);
        assert_eq!(header(Level::Best), [0x78, 0xDA]);
    }

    #[test]
    fn always_divisible_by_31() {
        for level in [Level::Store, Level::Fast, Level::Default, Level::Best] {
            let h = header(level);
            let value = u16::from_be_bytes(h);
            assert_eq!(value % 31, 0, "{level:?} -> {h:02X?}");
            assert_eq!(h[0], 0x78, "CMF must be 0x78");
        }
    }
}
