//! Lossless JPEG (ITU-T T.81 process 14, SOF3) — the standard DNG raw compression (`Compression =
//! 7`).
//!
//! This is the Huffman-coded, prediction-based *lossless* JPEG (not the lossy DCT codec). Each
//! sample is predicted from its left neighbour (predictor 1, with the spec's first-row/column
//! defaults), and the prediction error is Huffman-coded by magnitude category plus mantissa bits,
//! exactly as a JPEG DC coefficient. The mosaic / linear planes map to JPEG components (one per
//! `SamplesPerPixel`), interleaved. Differences are reduced modulo 2^16 so they always fit a
//! category `0..=16`, and reconstruction wraps to match the reference decoder.

use gamut_core::{Error, Result};

/// A decoded lossless-JPEG frame.
pub(crate) struct LosslessJpeg {
    pub width: usize,
    pub height: usize,
    pub components: usize,
    pub samples: Vec<u16>,
}

// JPEG markers.
const MARKER: u8 = 0xFF;
const SOI: u8 = 0xD8;
const EOI: u8 = 0xD9;
const SOF3: u8 = 0xC3;
const DHT: u8 = 0xC4;
const SOS: u8 = 0xDA;

/// A fixed, valid Huffman table over the 17 magnitude categories (`SSSS = 0..=16`).
///
/// `BITS[i]` (1-indexed) is the number of codes of length `i`; here 15 codes of length 4 and 2 of
/// length 5 (Kraft sum `15/16 + 2/32 = 1`). The table is written into the DHT, so the decoder uses
/// whatever we emit — correctness does not depend on it being optimal.
const BITS: [u8; 16] = [0, 0, 0, 15, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
const HUFFVAL: [u8; 17] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];

/// Canonical Huffman codes per symbol: `(code, length)` keyed by symbol value.
fn canonical_codes(bits: &[u8; 16], huffval: &[u8]) -> Vec<(u16, u8)> {
    // Code lengths in HUFFVAL order (JPEG Annex C: generate_size_table).
    let mut sizes = Vec::new();
    for (len_minus_1, &count) in bits.iter().enumerate() {
        for _ in 0..count {
            sizes.push((len_minus_1 + 1) as u8);
        }
    }
    // Codes (generate_code_table): ascending within each length.
    let mut table = vec![(0u16, 0u8); 256];
    let mut code: u16 = 0;
    let mut k = 0;
    let mut length = sizes.first().copied().unwrap_or(0);
    while k < sizes.len() {
        while k < sizes.len() && sizes[k] == length {
            table[huffval[k] as usize] = (code, length);
            code += 1;
            k += 1;
        }
        code <<= 1;
        length += 1;
    }
    table
}

/// MSB-first bit writer with JPEG `FF` → `FF 00` byte stuffing.
struct BitWriter {
    out: Vec<u8>,
    acc: u32,
    nbits: u32,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            acc: 0,
            nbits: 0,
        }
    }

    fn put(&mut self, value: u32, count: u8) {
        if count == 0 {
            return;
        }
        self.acc |= (value & ((1u32 << count) - 1)) << (32 - self.nbits - u32::from(count));
        self.nbits += u32::from(count);
        while self.nbits >= 8 {
            let byte = (self.acc >> 24) as u8;
            self.out.push(byte);
            if byte == MARKER {
                self.out.push(0x00); // stuff
            }
            self.acc <<= 8;
            self.nbits -= 8;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.nbits > 0 {
            // Pad the final partial byte with 1-bits (JPEG convention).
            self.put(0xFF, (8 - (self.nbits % 8)) as u8 % 8);
        }
        self.out
    }
}

/// The magnitude category (`SSSS`) of a difference and its mantissa bits.
fn magnitude(diff: i32) -> (u8, u32) {
    if diff == 0 {
        return (0, 0);
    }
    if diff == -32768 {
        return (16, 0); // T.81 special case: category 16 carries no mantissa.
    }
    let magnitude = diff.unsigned_abs();
    let ssss = (32 - magnitude.leading_zeros()) as u8;
    // Mantissa: diff for non-negative, diff - 1 (i.e. one's-complement low bits) for negative.
    let mantissa = if diff >= 0 { diff } else { diff - 1 } as u32;
    (ssss, mantissa & ((1u32 << ssss) - 1))
}

/// Reduces a raw prediction error to the canonical `[-32768, 32767]` range (mod 2^16).
fn reduce(diff: i32) -> i32 {
    if diff < -32768 {
        diff + 65536
    } else if diff > 32767 {
        diff - 65536
    } else {
        diff
    }
}

/// The predictor-1 prediction for sample at `(x, y)` of component `c`: the left neighbour, or the
/// sample above for the first column, or `2^(precision-1)` for the very first sample (T.81 H.1.2.1).
fn predict(
    samples: &[u16],
    width: usize,
    comp: usize,
    x: usize,
    y: usize,
    c: usize,
    precision: u16,
) -> i32 {
    let at = |xx: usize, yy: usize| i32::from(samples[(yy * width + xx) * comp + c]);
    if x > 0 {
        at(x - 1, y)
    } else if y > 0 {
        at(x, y - 1)
    } else {
        1i32 << (precision - 1)
    }
}

/// Encodes interleaved `samples` (`width * height * components`) as a lossless JPEG at `precision`
/// bits per sample.
#[must_use]
pub(crate) fn encode(
    samples: &[u16],
    width: usize,
    height: usize,
    components: usize,
    precision: u16,
) -> Vec<u8> {
    let codes = canonical_codes(&BITS, &HUFFVAL);
    let mut out = Vec::new();
    out.extend_from_slice(&[MARKER, SOI]);

    // SOF3: lossless frame.
    out.extend_from_slice(&[MARKER, SOF3]);
    let sof_len = 8 + 3 * components;
    out.extend_from_slice(&(sof_len as u16).to_be_bytes());
    out.push(precision as u8);
    out.extend_from_slice(&(height as u16).to_be_bytes());
    out.extend_from_slice(&(width as u16).to_be_bytes());
    out.push(components as u8);
    for c in 0..components {
        out.push((c + 1) as u8); // component id
        out.push(0x11); // H=1, V=1
        out.push(0x00); // quantization table (unused in lossless)
    }

    // DHT: one DC table (class 0, id 0).
    out.extend_from_slice(&[MARKER, DHT]);
    let dht_len = 2 + 1 + 16 + HUFFVAL.len();
    out.extend_from_slice(&(dht_len as u16).to_be_bytes());
    out.push(0x00); // Tc=0 (DC/lossless), Th=0
    out.extend_from_slice(&BITS);
    out.extend_from_slice(&HUFFVAL);

    // SOS: all components, predictor selector 1.
    out.extend_from_slice(&[MARKER, SOS]);
    let sos_len = 6 + 2 * components;
    out.extend_from_slice(&(sos_len as u16).to_be_bytes());
    out.push(components as u8);
    for c in 0..components {
        out.push((c + 1) as u8); // component selector
        out.push(0x00); // Td=0, Ta=0
    }
    out.push(1); // Ss = predictor 1
    out.push(0); // Se
    out.push(0); // Ah/Al

    let mut writer = BitWriter::new();
    for y in 0..height {
        for x in 0..width {
            for c in 0..components {
                let actual = i32::from(samples[(y * width + x) * components + c]);
                let diff = reduce(actual - predict(samples, width, components, x, y, c, precision));
                let (ssss, mantissa) = magnitude(diff);
                let (code, len) = codes[ssss as usize];
                writer.put(u32::from(code), len);
                if ssss > 0 && ssss < 16 {
                    writer.put(mantissa, ssss);
                }
            }
        }
    }
    out.extend_from_slice(&writer.finish());
    out.extend_from_slice(&[MARKER, EOI]);
    out
}

/// MSB-first bit reader that unstuffs `FF 00` and treats other markers as end-of-stream.
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    acc: u32,
    nbits: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            acc: 0,
            nbits: 0,
        }
    }

    fn next_bit(&mut self) -> u32 {
        if self.nbits == 0 {
            let byte = self.data.get(self.pos).copied().unwrap_or(0);
            self.pos += 1;
            if byte == MARKER {
                // FF 00 is a stuffed FF; any other follower is a marker (end of scan) — feed zeros.
                if self.data.get(self.pos) == Some(&0x00) {
                    self.pos += 1;
                } else {
                    self.pos -= 1; // leave the marker in place
                }
            }
            self.acc = u32::from(byte);
            self.nbits = 8;
        }
        self.nbits -= 1;
        (self.acc >> self.nbits) & 1
    }

    fn receive(&mut self, count: u8) -> u32 {
        let mut v = 0u32;
        for _ in 0..count {
            v = (v << 1) | self.next_bit();
        }
        v
    }
}

/// Decodes one Huffman symbol (the magnitude category) using the canonical table.
fn decode_symbol(reader: &mut BitReader, codes: &[(u16, u8)]) -> Result<u8> {
    let mut code: u16 = 0;
    for len in 1..=16u8 {
        code = (code << 1) | reader.next_bit() as u16;
        for (sym, &(c, l)) in codes.iter().enumerate() {
            if l == len && c == code {
                return Ok(sym as u8);
            }
        }
    }
    Err(Error::InvalidInput(
        "DNG: invalid lossless-JPEG Huffman code",
    ))
}

/// Reconstructs a difference from its category `ssss` and the mantissa bits read from `reader`.
fn extend(reader: &mut BitReader, ssss: u8) -> i32 {
    if ssss == 0 {
        return 0;
    }
    if ssss == 16 {
        return -32768;
    }
    let t = reader.receive(ssss) as i32;
    if t < (1 << (ssss - 1)) {
        t - (1 << ssss) + 1
    } else {
        t
    }
}

/// Reads a big-endian `u16` at `pos`.
fn be16(data: &[u8], pos: usize) -> Result<usize> {
    let b = data
        .get(pos..pos + 2)
        .ok_or(Error::InvalidInput("DNG: truncated lossless-JPEG marker"))?;
    Ok(usize::from(u16::from_be_bytes([b[0], b[1]])))
}

/// Decodes a lossless JPEG, returning its geometry and interleaved samples.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if the markers are malformed or the entropy stream is invalid.
pub(crate) fn decode(data: &[u8]) -> Result<LosslessJpeg> {
    if data.len() < 2 || data[0] != MARKER || data[1] != SOI {
        return Err(Error::InvalidInput("DNG: not a JPEG (missing SOI)"));
    }
    let mut pos = 2;
    let (mut width, mut height, mut components, mut precision) = (0usize, 0usize, 0usize, 0u16);
    let mut codes: Option<Vec<(u16, u8)>> = None;

    loop {
        // Find the next marker.
        while data.get(pos) != Some(&MARKER) {
            pos = pos
                .checked_add(1)
                .filter(|&p| p < data.len())
                .ok_or(Error::InvalidInput("DNG: lossless JPEG missing SOS"))?;
        }
        while data.get(pos) == Some(&MARKER) {
            pos += 1;
        }
        let marker = *data
            .get(pos)
            .ok_or(Error::InvalidInput("DNG: truncated lossless JPEG"))?;
        pos += 1;
        match marker {
            SOF3 => {
                let len = be16(data, pos)?;
                precision = u16::from(
                    *data
                        .get(pos + 2)
                        .ok_or(Error::InvalidInput("DNG: truncated SOF3"))?,
                );
                height = be16(data, pos + 3)?;
                width = be16(data, pos + 5)?;
                components = usize::from(
                    *data
                        .get(pos + 7)
                        .ok_or(Error::InvalidInput("DNG: truncated SOF3"))?,
                );
                pos += len;
            }
            DHT => {
                let len = be16(data, pos)?;
                // Single table assumed (class/id byte then BITS then HUFFVAL).
                let bits_start = pos + 3;
                let bits: [u8; 16] = data
                    .get(bits_start..bits_start + 16)
                    .ok_or(Error::InvalidInput("DNG: truncated DHT"))?
                    .try_into()
                    .unwrap();
                let nvals: usize = bits.iter().map(|&b| usize::from(b)).sum();
                let huffval = data
                    .get(bits_start + 16..bits_start + 16 + nvals)
                    .ok_or(Error::InvalidInput("DNG: truncated DHT values"))?;
                codes = Some(canonical_codes(&bits, huffval));
                pos += len;
            }
            SOS => {
                let len = be16(data, pos)?;
                pos += len; // skip the scan header; entropy data follows
                break;
            }
            EOI => return Err(Error::InvalidInput("DNG: lossless JPEG ended before SOS")),
            _ => {
                // Skip any other marker segment by its length.
                let len = be16(data, pos)?;
                pos += len;
            }
        }
    }

    let codes = codes.ok_or(Error::InvalidInput("DNG: lossless JPEG missing DHT"))?;
    if width == 0 || height == 0 || components == 0 {
        return Err(Error::InvalidInput(
            "DNG: lossless JPEG has zero dimensions",
        ));
    }
    let count = width
        .checked_mul(height)
        .and_then(|n| n.checked_mul(components))
        .ok_or(Error::InvalidInput(
            "DNG: lossless JPEG dimensions overflow",
        ))?;
    let mut samples = vec![0u16; count];
    let mut reader = BitReader::new(&data[pos..]);
    for y in 0..height {
        for x in 0..width {
            for c in 0..components {
                let ssss = decode_symbol(&mut reader, &codes)?;
                if ssss > 16 {
                    return Err(Error::InvalidInput(
                        "DNG: lossless-JPEG category out of range",
                    ));
                }
                let diff = extend(&mut reader, ssss);
                let px = predict(&samples, width, components, x, y, c, precision);
                samples[(y * width + x) * components + c] = (px + diff) as u16;
            }
        }
    }
    Ok(LosslessJpeg {
        width,
        height,
        components,
        samples,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(width: usize, height: usize, components: usize, precision: u16) {
        let max = (1u32 << precision) - 1;
        let samples: Vec<u16> = (0..width * height * components)
            .map(|i| ((i as u32).wrapping_mul(2654435761) % (max + 1)) as u16)
            .collect();
        let encoded = encode(&samples, width, height, components, precision);
        let decoded = decode(&encoded).expect("decode");
        assert_eq!(
            (decoded.width, decoded.height, decoded.components),
            (width, height, components)
        );
        assert_eq!(
            decoded.samples, samples,
            "{width}x{height}x{components} @ {precision}-bit"
        );
    }

    #[test]
    fn roundtrips_varied_shapes_and_depths() {
        for &precision in &[8u16, 12, 14, 16] {
            roundtrip(17, 9, 1, precision); // CFA-like single component, odd dims
            roundtrip(8, 8, 3, precision); // linear RGB
            roundtrip(1, 1, 1, precision); // smallest
        }
    }
}
