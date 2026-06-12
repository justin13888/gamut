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
            if next_code == (1 << width) && width < MAX_WIDTH {
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
    let mut out = Vec::with_capacity(expected);
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
        let entry = if (code as usize) < table.len() {
            table[code as usize].clone()
        } else if code as usize == table.len() {
            // `KwKwK`: the code names the entry being defined this step.
            let p = prev.ok_or(Error::InvalidInput("LZW: code before ClearCode"))? as usize;
            let mut s = table[p].clone();
            s.push(table[p][0]);
            s
        } else {
            return Err(Error::InvalidInput("LZW: code out of range"));
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
        return Err(Error::InvalidInput(
            "LZW: decoded fewer bytes than expected",
        ));
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
}
