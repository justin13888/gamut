//! PackBits compression (TIFF 6.0 §9, `Compression = 32773`).
//!
//! PackBits is a simple byte-oriented run-length scheme. A control byte `n` (read as a signed
//! `i8`) means: `0..=127` → copy the next `n + 1` bytes literally; `-127..=-1` → copy the next
//! single byte `1 - n` times; `-128` → no-op. Each image row is packed independently (runs never
//! cross a row boundary), so the encoder works one row at a time.

use gamut_core::{Error, Result};

/// Returns the length (capped at 128) of the run of bytes equal to `data[i]` starting at `i`.
fn run_length(data: &[u8], i: usize) -> usize {
    let b = data[i];
    let mut len = 1;
    while i + len < data.len() && data[i + len] == b && len < 128 {
        len += 1;
    }
    len
}

/// PackBits-encodes one row, appending to `out`. Runs never cross the row boundary (§9).
pub fn encode_row(row: &[u8], out: &mut Vec<u8>) {
    let mut i = 0;
    while i < row.len() {
        let run = run_length(row, i);
        if run >= 2 {
            // Replicate run: control = -(run - 1), stored as a `u8`.
            out.push((1i32 - run as i32) as i8 as u8);
            out.push(row[i]);
            i += run;
        } else {
            // Literal run: bytes up to the next run of ≥2, capped at 128.
            let start = i;
            while i < row.len() && i - start < 128 && run_length(row, i) < 2 {
                i += 1;
            }
            out.push((i - start - 1) as u8);
            out.extend_from_slice(&row[start..i]);
        }
    }
}

/// PackBits-decodes `src` until exactly `expected` bytes are produced.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if `src` is truncated or decodes to the wrong length.
pub fn decode(src: &[u8], expected: usize) -> Result<Vec<u8>> {
    // Cap the pre-allocation so a malformed `expected` can't reserve a huge buffer up front.
    let mut out = Vec::with_capacity(expected.min(1 << 16));
    let mut i = 0;
    while out.len() < expected {
        let n = *src
            .get(i)
            .ok_or(Error::InvalidInput("PackBits: truncated control byte"))? as i8;
        i += 1;
        if n >= 0 {
            let count = n as usize + 1;
            let chunk = src
                .get(i..i + count)
                .ok_or(Error::InvalidInput("PackBits: truncated literal run"))?;
            out.extend_from_slice(chunk);
            i += count;
        } else if n != -128 {
            let count = (1 - i32::from(n)) as usize;
            let b = *src
                .get(i)
                .ok_or(Error::InvalidInput("PackBits: truncated replicate run"))?;
            i += 1;
            out.resize(out.len() + count, b);
        }
    }
    if out.len() != expected {
        return Err(Error::InvalidInput("PackBits: decoded length mismatch"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(row: &[u8]) {
        let mut enc = Vec::new();
        encode_row(row, &mut enc);
        let dec = decode(&enc, row.len()).expect("decode");
        assert_eq!(dec, row);
    }

    #[test]
    fn roundtrips_runs_and_literals() {
        roundtrip(&[]);
        roundtrip(&[5]);
        roundtrip(&[7, 7, 7, 7, 7]);
        roundtrip(&[1, 2, 3, 4, 5]);
        roundtrip(&[9, 9, 1, 2, 9, 9, 9, 3]);
        roundtrip(&[0xAA; 300]); // run longer than 128
        let mixed: Vec<u8> = (0..200).map(|i| (i % 7) as u8).collect();
        roundtrip(&mixed);
        roundtrip(&[42; 128]);
        roundtrip(&[42; 129]);
    }

    #[test]
    fn decode_rejects_truncation_and_overrun() {
        assert!(decode(&[], 4).is_err());
        assert!(decode(&[0x00], 4).is_err()); // literal of 1 but no data byte
        // A literal claiming 5 bytes when only 4 are expected overruns.
        assert!(decode(&[0x04, 1, 2, 3, 4, 5], 4).is_err());
    }
}
