//! CCITT T.6 (Group 4) two-dimensional bilevel coding (TIFF 6.0 §11, `Compression = 4`).
//!
//! Every row is coded relative to the row above it (the first row's reference is an imaginary
//! all-white line). Each coding step picks one of three modes — pass, horizontal, or vertical —
//! from the positions of the next changing elements `a1`/`a2` on the coding line and `b1`/`b2` on
//! the reference line, exactly as in CCITT T.4 two-dimensional coding. There are no EOL codes; the
//! stream ends with an End-of-Facsimile-Block. Run lengths in horizontal mode reuse the T.4
//! make-up/terminating tables from [`super`].

use gamut_bitstream::BitWriter;
use gamut_core::{Error, Result};

use super::{BitReader, decode_run, encode_run};

/// The two End-of-Line codes that form an End-of-Facsimile-Block.
const EOL: u32 = 0b0000_0000_0001;

/// Positions of the changing elements of a packed row (where the colour differs from the pixel to
/// the left; the imaginary pixel before column 0 is white).
fn changing_elements(row: &[u8], width: usize) -> Vec<usize> {
    let mut t = Vec::new();
    let mut prev = 0u8;
    for x in 0..width {
        let bit = (row[x / 8] >> (7 - (x % 8))) & 1;
        if bit != prev {
            t.push(x);
            prev = bit;
        }
    }
    t
}

/// `a1`/`a2`: the next two changing elements on the coding line strictly to the right of `a0`.
fn coding_changes(cod: &[usize], a0: i32, width: usize) -> (usize, usize) {
    let mut k = 0;
    while k < cod.len() && (cod[k] as i32) <= a0 {
        k += 1;
    }
    let a1 = cod.get(k).copied().unwrap_or(width);
    let a2 = cod.get(k + 1).copied().unwrap_or(width);
    (a1, a2)
}

/// `b1`/`b2`: the first changing element on the reference line to the right of `a0` and of the
/// colour opposite `a0`, and the one after it.
fn reference_changes(refr: &[usize], a0: i32, a0_color: u8, width: usize) -> (usize, usize) {
    let opposite = 1 - a0_color;
    let mut k = 0;
    while k < refr.len() && (refr[k] as i32) <= a0 {
        k += 1;
    }
    // The colour to the right of `refr[k]` is black when `k` is even. Skip one if it doesn't match.
    if k < refr.len() {
        let color = u8::from(k % 2 == 0);
        if color != opposite {
            k += 1;
        }
    }
    let b1 = refr.get(k).copied().unwrap_or(width);
    let b2 = refr.get(k + 1).copied().unwrap_or(width);
    (b1, b2)
}

/// Writes a vertical-mode code for the offset `d` (|d| ≤ 3).
fn put_vertical(out: &mut BitWriter, d: i32) {
    match d {
        0 => out.put_bits(0b1, 1),
        1 => out.put_bits(0b011, 3),
        -1 => out.put_bits(0b010, 3),
        2 => out.put_bits(0b000011, 6),
        -2 => out.put_bits(0b000010, 6),
        3 => out.put_bits(0b0000011, 7),
        _ => out.put_bits(0b0000010, 7), // -3
    }
}

/// Two-dimensionally encodes one coding row against `refr` into `out`.
fn encode_row(out: &mut BitWriter, cod: &[usize], refr: &[usize], width: usize) -> Result<()> {
    let mut a0: i32 = -1;
    let mut a0_color = 0u8;
    while a0 < width as i32 {
        let (a1, a2) = coding_changes(cod, a0, width);
        let (b1, b2) = reference_changes(refr, a0, a0_color, width);
        if b2 < a1 {
            out.put_bits(0b0001, 4); // pass mode
            a0 = b2 as i32;
        } else if (a1 as i32 - b1 as i32).abs() <= 3 {
            put_vertical(out, a1 as i32 - b1 as i32);
            a0 = a1 as i32;
            a0_color = 1 - a0_color;
        } else {
            out.put_bits(0b001, 3); // horizontal mode
            let start = a0.max(0) as usize;
            encode_run(out, a1 - start, a0_color == 0)?;
            encode_run(out, a2 - a1, a0_color != 0)?;
            a0 = a2 as i32;
        }
    }
    Ok(())
}

/// T.6-encodes a strip of `rows` packed bilevel rows.
///
/// # Errors
///
/// Returns [`Error::Unsupported`] only on an internal table inconsistency (never for valid input).
pub fn g4_encode_strip(
    packed: &[u8],
    stored_row_bytes: usize,
    rows: usize,
    width: usize,
) -> Result<Vec<u8>> {
    let mut out = BitWriter::new();
    let mut refr: Vec<usize> = Vec::new();
    for r in 0..rows {
        let row = &packed[r * stored_row_bytes..(r + 1) * stored_row_bytes];
        let cod = changing_elements(row, width);
        encode_row(&mut out, &cod, &refr, width)?;
        refr = cod;
    }
    out.put_bits(EOL, 12);
    out.put_bits(EOL, 12);
    out.byte_align();
    Ok(out.into_bytes())
}

/// Sets pixels `[from, to)` of `dst` to `color` (only black needs writing; `dst` starts white).
fn fill(dst: &mut [u8], from: usize, to: usize, color: u8, width: usize) {
    if color == 1 {
        for p in from..to.min(width) {
            dst[p / 8] |= 0x80 >> (p % 8);
        }
    }
}

/// A decoded 2D mode: pass, horizontal, or vertical with a signed offset in `-3..=3`.
enum Mode {
    Pass,
    Horizontal,
    Vertical(i32),
}

fn read_mode(r: &mut BitReader) -> Result<Mode> {
    let mut bit = || {
        r.read_bit()
            .ok_or(Error::InvalidInput("CCITT: truncated mode code"))
    };
    if bit()? == 1 {
        return Ok(Mode::Vertical(0));
    }
    // 0…
    if bit()? == 1 {
        // 01x
        return Ok(Mode::Vertical(if bit()? == 1 { 1 } else { -1 }));
    }
    if bit()? == 1 {
        return Ok(Mode::Horizontal); // 001
    }
    if bit()? == 1 {
        return Ok(Mode::Pass); // 0001
    }
    if bit()? == 1 {
        // 00001x
        return Ok(Mode::Vertical(if bit()? == 1 { 2 } else { -2 }));
    }
    if bit()? == 1 {
        // 000001x
        return Ok(Mode::Vertical(if bit()? == 1 { 3 } else { -3 }));
    }
    Err(Error::InvalidInput(
        "CCITT: unsupported 2D mode (EOFB or extension)",
    ))
}

/// Two-dimensionally decodes one row into `dst`, returning its changing elements (the next
/// reference line).
fn decode_row(
    r: &mut BitReader,
    refr: &[usize],
    width: usize,
    dst: &mut [u8],
) -> Result<Vec<usize>> {
    let mut a0: i32 = -1;
    let mut a0_color = 0u8;
    while a0 < width as i32 {
        let (b1, b2) = reference_changes(refr, a0, a0_color, width);
        let start = a0.max(0) as usize;
        match read_mode(r)? {
            Mode::Pass => {
                fill(dst, start, b2, a0_color, width);
                a0 = b2 as i32;
            }
            Mode::Horizontal => {
                let run1 = decode_run(r, a0_color == 0)?;
                let run2 = decode_run(r, a0_color != 0)?;
                let a1 = (start + run1).min(width);
                let a2 = (a1 + run2).min(width);
                fill(dst, start, a1, a0_color, width);
                fill(dst, a1, a2, 1 - a0_color, width);
                a0 = a2 as i32;
            }
            Mode::Vertical(d) => {
                let a1 = (b1 as i32 + d).clamp(0, width as i32) as usize;
                fill(dst, start, a1, a0_color, width);
                a0 = a1 as i32;
                a0_color = 1 - a0_color;
            }
        }
    }
    Ok(changing_elements(dst, width))
}

/// T.6-decodes a strip into `rows` packed bilevel rows of `width` pixels each.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if the stream is truncated or a code is invalid.
pub fn g4_decode_strip(data: &[u8], rows: usize, width: usize) -> Result<Vec<u8>> {
    let stored = width.div_ceil(8);
    let mut reader = BitReader::new(data);
    let mut out = vec![0u8; stored * rows];
    let mut refr: Vec<usize> = Vec::new();
    for r in 0..rows {
        let dst = &mut out[r * stored..(r + 1) * stored];
        refr = decode_row(&mut reader, &refr, width, dst)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(bits: &[u8]) -> Vec<u8> {
        let mut row = vec![0u8; bits.len().div_ceil(8)];
        for (i, &b) in bits.iter().enumerate() {
            if b != 0 {
                row[i / 8] |= 0x80 >> (i % 8);
            }
        }
        row
    }

    fn roundtrip(rows: &[Vec<u8>], width: usize) {
        let stored = width.div_ceil(8);
        let packed: Vec<u8> = rows.iter().flatten().copied().collect();
        let enc = g4_encode_strip(&packed, stored, rows.len(), width).expect("encode");
        let dec = g4_decode_strip(&enc, rows.len(), width).expect("decode");
        assert_eq!(dec, packed);
    }

    #[test]
    fn roundtrips_patterns() {
        let w = 24;
        roundtrip(&[pack(&[0; 24])], w);
        roundtrip(&[pack(&[1; 24])], w);
        let alt: Vec<u8> = (0..24).map(|i| (i % 2) as u8).collect();
        roundtrip(&[pack(&alt), pack(&alt), pack(&alt)], w);
        // Vertical/pass coding shines when rows resemble their predecessor.
        let a: Vec<u8> = (0..24).map(|i| u8::from((6..18).contains(&i))).collect();
        let b: Vec<u8> = (0..24).map(|i| u8::from((5..17).contains(&i))).collect();
        roundtrip(&[pack(&a), pack(&b), pack(&a), pack(&b)], w);
        // Wide rows with long runs (make-up codes in horizontal mode).
        let mut wide = vec![0u8; 300];
        for x in wide.iter_mut().take(200).skip(50) {
            *x = 1;
        }
        roundtrip(&[pack(&wide), pack(&wide)], 300);
    }
}
