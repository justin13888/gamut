//! LZW compression (TIFF 6.0 §13, `Compression = 5`).
//!
//! TIFF LZW codes the *bytes* of a strip (whatever the bit depth) with variable-width codes,
//! 9 to 12 bits, MSB-first (`FillOrder = 1`). Code 256 is `ClearCode`, 257 is `EndOfInformation`,
//! and the first string-table entry is 258. Each strip begins with a `ClearCode` and ends with an
//! `EndOfInformation`. Following TIFF's "early change" convention, the encoder widens the code one
//! step before the table fills (at 2^width) and the decoder — which lags one entry — widens at
//! 2^width − 1.

use gamut_bitstream::BitWriter;
use gamut_core::{Error, Result};

const CLEAR: u32 = 256;
const EOI: u32 = 257;
const FIRST: u32 = 258;
const MAX_WIDTH: u32 = 12;
/// The table is reset when the next free code reaches this value (one before the 12-bit limit).
const RESET_AT: u32 = 4094;

/// The table is cleared before a code could ever need a thirteenth bit.
///
/// `encode` widens at `next_code == 1 << width`, so the widths step 9 -> 10 -> 11 -> 12 at 512,
/// 1024 and 2048. Reaching `1 << MAX_WIDTH` would ask for width 13, and this assertion is why that
/// cannot happen: the reset fires first. It replaces a runtime `width < MAX_WIDTH` guard that
/// could never be false when it was evaluated -- an equivalent mutant no test could kill (#110) --
/// with a relationship the compiler checks once.
const _: () = assert!(RESET_AT < (1 << MAX_WIDTH));

/// LZW-encodes `data` (one strip's bytes) into a self-delimiting `ClearCode … EndOfInformation`
/// stream.
#[must_use]
pub fn encode(data: &[u8]) -> Vec<u8> {
    use std::collections::HashMap;

    let mut out = BitWriter::new();
    let mut width = 9u32;
    out.put_bits(CLEAR, width);

    let Some((&first, rest)) = data.split_first() else {
        out.put_bits(EOI, width);
        return out.into_bytes();
    };

    let mut table: HashMap<(u32, u8), u32> = HashMap::new();
    let mut next_code = FIRST;
    let mut omega = u32::from(first);
    for &k in rest {
        if let Some(&code) = table.get(&(omega, k)) {
            omega = code;
        } else {
            out.put_bits(omega, width);
            table.insert((omega, k), next_code);
            next_code += 1;
            if next_code == (1 << width) {
                width += 1;
            }
            if next_code == RESET_AT {
                out.put_bits(CLEAR, width);
                table.clear();
                next_code = FIRST;
                width = 9;
            }
            omega = u32::from(k);
        }
    }
    out.put_bits(omega, width);
    out.put_bits(EOI, width);
    out.into_bytes()
}

/// A MSB-first bit reader over LZW-coded bytes.
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl BitReader<'_> {
    fn read(&mut self, n: u32) -> Option<u32> {
        let mut value = 0u32;
        for _ in 0..n {
            let byte = *self.data.get(self.pos / 8)?;
            value = (value << 1) | u32::from((byte >> (7 - (self.pos % 8))) & 1);
            self.pos += 1;
        }
        Some(value)
    }
}

/// Builds the initial string table: 256 single bytes plus the two reserved codes.
fn init_table() -> Vec<Vec<u8>> {
    let mut table: Vec<Vec<u8>> = (0..=255u32).map(|b| vec![b as u8]).collect();
    table.push(Vec::new()); // 256 ClearCode (unused as a string)
    table.push(Vec::new()); // 257 EndOfInformation
    table
}

/// LZW-decodes a strip into exactly `expected` bytes.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if the stream is truncated, a code is out of range, or the
/// output is shorter than `expected`.
pub fn decode(data: &[u8], expected: usize) -> Result<Vec<u8>> {
    let mut reader = BitReader { data, pos: 0 };
    // Cap the pre-allocation so a malformed `expected` can't reserve a huge buffer up front; the
    // buffer still grows as the (input-bounded) decode produces bytes.
    let mut out = Vec::with_capacity(expected.min(1 << 16));
    let mut table = init_table();
    let mut width = 9u32;
    let mut prev: Option<u32> = None;

    while let Some(code) = reader.read(width) {
        if code == EOI {
            break;
        }
        if code == CLEAR {
            table = init_table();
            width = 9;
            prev = None;
            continue;
        }
        // Indexing invariant: every `table` entry is non-empty — `init_table` seeds the 256
        // single-byte strings (plus the reserved Clear/EOI slots, which are never reached here:
        // Clear/EOI are handled above and `prev` is reset to None on Clear, so `p` and the
        // in-range `code` always name a seeded or appended entry), and every appended entry is a
        // clone with one byte pushed. `table[p][0]` / `entry[0]` therefore cannot panic.
        let entry = if (code as usize) < table.len() {
            table[code as usize].clone()
        } else if code as usize == table.len() {
            // `KwKwK`: the code names the entry being defined this step.
            let p = prev.ok_or_else(|| {
                Error::invalid_input(env!("CARGO_PKG_NAME"), "LZW: code before ClearCode")
                    .with_byte_offset((reader.pos / 8) as u64)
            })? as usize;
            let mut s = table[p].clone();
            s.push(table[p][0]);
            s
        } else {
            return Err(
                Error::invalid_input(env!("CARGO_PKG_NAME"), "LZW: code out of range")
                    .with_byte_offset((reader.pos / 8) as u64),
            );
        };
        out.extend_from_slice(&entry);

        if let Some(p) = prev {
            let mut s = table[p as usize].clone();
            s.push(entry[0]);
            table.push(s);
            if table.len() == ((1 << width) - 1) as usize && width < MAX_WIDTH {
                width += 1;
            }
        }
        prev = Some(code);
    }

    if out.len() < expected {
        return Err(Error::invalid_input(
            env!("CARGO_PKG_NAME"),
            "LZW: decoded fewer bytes than expected",
        )
        .with_byte_offset((reader.pos / 8) as u64));
    }
    out.truncate(expected);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(data: &[u8]) {
        let enc = encode(data);
        let dec = decode(&enc, data.len()).expect("decode");
        assert_eq!(dec, data);
    }

    /// Repetitive input actually compresses -- the encoder's whole purpose.
    ///
    /// Every other LZW test is a round trip or a libtiff differential, and none of them can see an
    /// encoder that has stopped compressing: the output stays valid LZW and decodes correctly, it
    /// is merely enormous. Mutating the table-reset test `next_code == RESET_AT` to `!=` fires a
    /// CLEAR after nearly every code, and the whole suite still passed (#110).
    ///
    /// Measured on this input: 8192 bytes in, **391 out** correctly, **18 434** under that mutant
    /// -- a compressor that more than doubles what it is given. The bound below is deliberately
    /// loose (4x, against the ~21x actually achieved), because this pins "compression happens at
    /// all", not a particular ratio a legitimate tuning change might move.
    #[test]
    fn repetitive_input_compresses() {
        let src: Vec<u8> = (0..8192).map(|i| (i % 7) as u8).collect();
        let enc = encode(&src);

        assert!(
            enc.len() * 4 < src.len(),
            "expected at least 4x compression, got {} -> {}",
            src.len(),
            enc.len()
        );
        assert_eq!(
            decode(&enc, src.len()).expect("decode"),
            src,
            "and it still decodes to the original"
        );
    }

    #[test]
    fn roundtrips_varied_inputs() {
        roundtrip(&[]);
        roundtrip(&[42]);
        roundtrip(&[1, 2, 3, 4, 5]);
        roundtrip(&[7, 7, 7, 7, 7, 7, 7, 7]);
        roundtrip(b"TOBEORNOTTOBEORTOBEORNOT");
        // Enough distinct strings to cross the 9->10->11->12-bit width boundaries and reset.
        let big: Vec<u8> = (0..20000u32)
            .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
            .collect();
        roundtrip(&big);
        roundtrip(&vec![0xABu8; 10000]);
    }

    fn codes(codes: &[u32]) -> Vec<u8> {
        let mut writer = BitWriter::new();
        for &code in codes {
            writer.put_bits(code, 9);
        }
        writer.into_bytes()
    }

    #[test]
    fn malformed_streams_report_the_consumed_byte_offset() {
        let mut before_clear = vec![CLEAR; 7];
        before_clear.push(FIRST);
        let error = decode(&codes(&before_clear), 1).unwrap_err();
        assert_eq!(error.static_message(), Some("LZW: code before ClearCode"));
        assert_eq!(error.byte_offset(), Some(9));

        let mut out_of_range = vec![CLEAR; 7];
        out_of_range.push(FIRST + 1);
        let error = decode(&codes(&out_of_range), 1).unwrap_err();
        assert_eq!(error.static_message(), Some("LZW: code out of range"));
        assert_eq!(error.byte_offset(), Some(9));

        let mut short = vec![CLEAR];
        short.extend(0..7);
        short.push(EOI);
        let error = decode(&codes(&short), 8).unwrap_err();
        assert_eq!(
            error.static_message(),
            Some("LZW: decoded fewer bytes than expected")
        );
        assert_eq!(error.byte_offset(), Some(10));
    }
}
