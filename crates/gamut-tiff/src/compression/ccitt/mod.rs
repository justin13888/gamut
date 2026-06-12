//! CCITT bilevel coding.
//!
//! This phase implements **Modified Huffman** (TIFF 6.0 §10, `Compression = 2`): each row is
//! coded as alternating white/black runs using the T.4 run-length tables ([`tables`]). A row
//! always begins with a (possibly zero-length) white run; a run is zero or more make-up codes
//! followed by exactly one terminating code; and each row starts on a byte boundary (no EOL, no
//! fill bits, no RTC). T.4/T.6 (Group 3/4) reuse these tables in a later phase.

mod tables;

use std::collections::HashMap;
use std::sync::OnceLock;

use gamut_bitstream::BitWriter;
use gamut_core::{Error, Result};

/// Parsed code tables: run → (codeword value, bit length) for encoding, and (bit length,
/// codeword value) → run for decoding, separately for white and black runs.
struct Codes {
    white_enc: HashMap<u16, (u32, u8)>,
    black_enc: HashMap<u16, (u32, u8)>,
    white_dec: HashMap<(u8, u32), u16>,
    black_dec: HashMap<(u8, u32), u16>,
}

/// Parses a MSB-first binary string into `(value, bit length)`.
fn parse(code: &str) -> (u32, u8) {
    let value = code
        .bytes()
        .fold(0u32, |acc, b| (acc << 1) | u32::from(b == b'1'));
    (value, code.len() as u8)
}

fn codes() -> &'static Codes {
    static CODES: OnceLock<Codes> = OnceLock::new();
    CODES.get_or_init(|| {
        let mut c = Codes {
            white_enc: HashMap::new(),
            black_enc: HashMap::new(),
            white_dec: HashMap::new(),
            black_dec: HashMap::new(),
        };
        for &(run, code) in tables::WHITE.iter().chain(tables::SHARED_MAKEUP) {
            let (v, l) = parse(code);
            c.white_enc.insert(run, (v, l));
            c.white_dec.insert((l, v), run);
        }
        for &(run, code) in tables::BLACK.iter().chain(tables::SHARED_MAKEUP) {
            let (v, l) = parse(code);
            c.black_enc.insert(run, (v, l));
            c.black_dec.insert((l, v), run);
        }
        c
    })
}

/// Reads `1` bit at `index` (MSB-first) from a packed row, or `0` past the end.
fn bit_at(row: &[u8], index: usize) -> u8 {
    (row[index / 8] >> (7 - (index % 8))) & 1
}

/// Emits one run of `length` pixels (zero or more make-up codes + one terminating code).
fn encode_run(out: &mut BitWriter, length: usize, white: bool) -> Result<()> {
    let codes = codes();
    let enc = if white {
        &codes.white_enc
    } else {
        &codes.black_enc
    };
    let put = |out: &mut BitWriter, run: u16| -> Result<()> {
        let &(v, l) = enc
            .get(&run)
            .ok_or(Error::Unsupported("CCITT: missing run code"))?;
        out.put_bits(v, u32::from(l));
        Ok(())
    };
    let mut r = length;
    while r >= 64 {
        let m = ((r / 64) * 64).min(2560);
        put(out, m as u16)?;
        r -= m;
    }
    put(out, r as u16)
}

/// Modified-Huffman-encodes one packed bilevel row of `width` pixels into `out`, then byte-aligns.
///
/// Following the CCITT convention (and libtiff), a `0` bit is a white pixel; the image's
/// photometric interpretation is applied separately when the bits become samples.
fn encode_row(out: &mut BitWriter, row: &[u8], width: usize) -> Result<()> {
    let white_bit = 0u8;
    let mut expect_white = true;
    let mut x = 0;
    while x < width {
        let is_white = bit_at(row, x) == white_bit;
        let start = x;
        while x < width && (bit_at(row, x) == white_bit) == is_white {
            x += 1;
        }
        // A row must start with a white run; insert a zero-length one if it starts black.
        if is_white != expect_white {
            encode_run(out, 0, expect_white)?;
            expect_white = !expect_white;
        }
        encode_run(out, x - start, is_white)?;
        expect_white = !expect_white;
    }
    out.byte_align();
    Ok(())
}

/// Modified-Huffman-encodes a strip of `packed` bilevel rows (`stored_row_bytes` each).
///
/// # Errors
///
/// Returns [`Error::Unsupported`] only on an internal table inconsistency (never for valid input).
pub fn mh_encode_strip(packed: &[u8], stored_row_bytes: usize, width: usize) -> Result<Vec<u8>> {
    let mut out = BitWriter::new();
    for row in packed.chunks(stored_row_bytes) {
        encode_row(&mut out, row, width)?;
    }
    Ok(out.into_bytes())
}

/// A MSB-first bit reader over a strip's coded bytes.
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl BitReader<'_> {
    fn read_bit(&mut self) -> Option<u8> {
        let byte = self.data.get(self.pos / 8)?;
        let bit = (byte >> (7 - (self.pos % 8))) & 1;
        self.pos += 1;
        Some(bit)
    }

    fn align_to_byte(&mut self) {
        self.pos = self.pos.div_ceil(8) * 8;
    }
}

/// Decodes one run-length code (white or black), returning its run length.
fn decode_code(r: &mut BitReader, white: bool) -> Result<u16> {
    let dec = if white {
        &codes().white_dec
    } else {
        &codes().black_dec
    };
    let mut value = 0u32;
    let mut len = 0u8;
    loop {
        let bit = r
            .read_bit()
            .ok_or(Error::InvalidInput("CCITT: truncated code"))?;
        value = (value << 1) | u32::from(bit);
        len += 1;
        if let Some(&run) = dec.get(&(len, value)) {
            return Ok(run);
        }
        if len >= 14 {
            return Err(Error::InvalidInput("CCITT: invalid code"));
        }
    }
}

/// Decodes a full run (make-up codes + one terminating code) of the given colour.
fn decode_run(r: &mut BitReader, white: bool) -> Result<usize> {
    let mut total = 0usize;
    loop {
        let run = decode_code(r, white)? as usize;
        total += run;
        if run < 64 {
            return Ok(total);
        }
    }
}

/// Modified-Huffman-decodes a strip into `rows` packed bilevel rows of `width` pixels each.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if the stream is truncated, a code is invalid, or a row's runs
/// do not sum to `width`.
pub fn mh_decode_strip(data: &[u8], rows: usize, width: usize) -> Result<Vec<u8>> {
    let stored_row_bytes = width.div_ceil(8);
    let white_bit = 0u8;
    let mut reader = BitReader { data, pos: 0 };
    let mut out = vec![0u8; stored_row_bytes * rows];
    for row in 0..rows {
        let dst = &mut out[row * stored_row_bytes..(row + 1) * stored_row_bytes];
        let mut pos = 0;
        let mut white = true;
        while pos < width {
            let run = decode_run(&mut reader, white)?;
            if pos + run > width {
                return Err(Error::InvalidInput("CCITT: run overruns the row"));
            }
            let bit = if white { white_bit } else { 1 - white_bit };
            if bit == 1 {
                for p in pos..pos + run {
                    dst[p / 8] |= 0x80 >> (p % 8);
                }
            }
            pos += run;
            white = !white;
        }
        reader.align_to_byte();
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(rows: &[Vec<u8>], width: usize) {
        let stored = width.div_ceil(8);
        let packed: Vec<u8> = rows.iter().flatten().copied().collect();
        let enc = mh_encode_strip(&packed, stored, width).expect("encode");
        let dec = mh_decode_strip(&enc, rows.len(), width).expect("decode");
        assert_eq!(dec, packed);
    }

    /// Packs a row of 0/1 pixel values into MSB-first bytes.
    fn pack(bits: &[u8]) -> Vec<u8> {
        let mut row = vec![0u8; bits.len().div_ceil(8)];
        for (i, &b) in bits.iter().enumerate() {
            if b != 0 {
                row[i / 8] |= 0x80 >> (i % 8);
            }
        }
        row
    }

    #[test]
    fn roundtrips_simple_rows() {
        // all white, all black, alternating, starts-black.
        let w = 16;
        roundtrip(&[pack(&[0; 16])], w);
        roundtrip(&[pack(&[1; 16])], w);
        let alt: Vec<u8> = (0..16).map(|i| (i % 2) as u8).collect();
        roundtrip(&[pack(&alt)], w);
        let starts_black: Vec<u8> = (0..16).map(|i| u8::from(i < 5)).collect();
        roundtrip(&[pack(&starts_black)], w);
    }

    #[test]
    fn roundtrips_long_runs() {
        // A run longer than 64 forces a make-up code; > 2623 forces repeated 2560 make-ups.
        let w = 3000;
        let mut bits = vec![0u8; w];
        for b in bits.iter_mut().take(2700) {
            *b = 1;
        }
        roundtrip(&[pack(&bits)], w);
    }
}
