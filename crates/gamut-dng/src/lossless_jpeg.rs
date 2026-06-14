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
        // `put` drains whole bytes, so `nbits` is always in `0..8` here.
        if self.nbits > 0 {
            // Pad the final partial byte with 1-bits (JPEG convention).
            self.put(0xFF, 8 - self.nbits as u8);
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
                // Categories 0 and 16 carry no mantissa; only `1..16` has magnitude bits.
                if ssss != 0 && ssss < 16 {
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
            // + (not |): v<<1 has a clear low bit, so | would be an equivalent (unkillable) mutant.
            v = (v << 1) + self.next_bit();
        }
        v
    }
}

/// Decodes one Huffman symbol (the magnitude category) using the canonical table.
fn decode_symbol(reader: &mut BitReader, codes: &[(u16, u8)]) -> Result<u8> {
    let mut code: u16 = 0;
    for len in 1..=16u8 {
        // + (not |): code<<1 has a clear low bit, so | would be an equivalent (unkillable) mutant.
        code = (code << 1) + reader.next_bit() as u16;
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
        // Find the next marker: skip to the first `FF` at or after `pos`. `position` returns an
        // offset strictly within `data[pos..]`, so there is no off-by-one bound to mutate.
        let off = data[pos..]
            .iter()
            .position(|&b| b == MARKER)
            .ok_or(Error::InvalidInput("DNG: lossless JPEG missing SOS"))?;
        pos += off;
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

    /// Round-trips an explicit sample buffer (so we can hand-pick values that exercise the
    /// Huffman category math: zero, small/large positive and negative DC differences, and the
    /// full 16-bit extremes that drive `reduce`/`magnitude`/`extend`).
    fn roundtrip_samples(samples: &[u16], width: usize, height: usize, components: usize) {
        let encoded = encode(samples, width, height, components, 16);
        let decoded = decode(&encoded).expect("decode");
        assert_eq!(decoded.width, width);
        assert_eq!(decoded.height, height);
        assert_eq!(decoded.components, components);
        assert_eq!(decoded.samples, samples);
    }

    #[test]
    fn roundtrips_extreme_dc_differences() {
        // A single row whose neighbour deltas hit the category boundaries (incl. wrap-around so
        // `reduce` produces -32768 / 32767) and both signs of every magnitude.
        let row: Vec<u16> = vec![
            0, 65535, 0, 1, 0, 32768, 32767, 0, 2, 4, 8, 16, 256, 65280, 65535, 0,
        ];
        let len = row.len();
        roundtrip_samples(&row, len, 1, 1);
        // Same values down a single column (exercises the first-column "predict from above" path).
        roundtrip_samples(&row, 1, len, 1);
        // Two interleaved components with offset values (exercises per-component prediction).
        let mut two = Vec::new();
        for &v in &row {
            two.push(v);
            two.push(v ^ 0x8000);
        }
        roundtrip_samples(&two, len, 1, 2);
    }

    #[test]
    fn reduce_wraps_at_exact_boundaries() {
        // Inside the canonical range: unchanged (kills `< -32768` -> `<= -32768`,
        // `> 32767` -> `>= 32767`).
        assert_eq!(reduce(-32768), -32768);
        assert_eq!(reduce(32767), 32767);
        // Just outside: wraps by exactly 2^16 (kills `> 32767` -> `==`, and the subtraction
        // operator mutants `-` -> `+` / `/`).
        assert_eq!(reduce(32768), -32768);
        assert_eq!(reduce(-32769), 32767);
        assert_eq!(reduce(40000), 40000 - 65536);
        assert_eq!(reduce(-40000), -40000 + 65536);
    }

    #[test]
    fn magnitude_categories_are_exact() {
        assert_eq!(magnitude(0), (0, 0));
        // The T.81 special case: -32768 is category 16 with no mantissa. Deleting the unary `-`
        // would make this `diff == 32768` (never reached) and fall through to (16, 0x7fff).
        assert_eq!(magnitude(-32768), (16, 0));
        // Positive: mantissa == diff; negative: one's-complement low bits (diff - 1).
        assert_eq!(magnitude(1), (1, 1));
        assert_eq!(magnitude(-1), (1, 0));
        assert_eq!(magnitude(2), (2, 0b10));
        assert_eq!(magnitude(-2), (2, 0b01));
        assert_eq!(magnitude(32767), (15, 0x7fff));
    }

    #[test]
    fn extend_reconstructs_category_16_as_min() {
        // ssss == 16 reads no mantissa and must yield -32768 (kills the deleted unary `-`, which
        // would return 32768; observable only here since the decoder folds both mod 2^16).
        let mut reader = BitReader::new(&[]);
        assert_eq!(extend(&mut reader, 16), -32768);
        // ssss == 0 yields 0 with no bits consumed.
        let mut reader = BitReader::new(&[]);
        assert_eq!(extend(&mut reader, 0), 0);
    }

    #[test]
    fn bit_writer_pads_only_a_partial_final_byte() {
        // Exactly one full byte: finish must NOT append a padding byte (kills `nbits > 0`
        // -> `nbits >= 0`, which would `put(0xff, 8)` and grow the output).
        let mut w = BitWriter::new();
        w.put(0b1010_1010, 8);
        assert_eq!(w.finish(), vec![0b1010_1010]);
        // A partial byte (3 bits) is padded with 1-bits up to one byte.
        let mut w = BitWriter::new();
        w.put(0b101, 3);
        assert_eq!(w.finish(), vec![0b1011_1111]);
        // Empty writer produces no bytes.
        assert_eq!(BitWriter::new().finish(), Vec::<u8>::new());
    }

    #[test]
    fn bit_reader_holds_position_at_a_real_marker() {
        // `FF` followed by a non-`00` byte is a real marker (end of scan): the reader must leave
        // it in place and feed `1`-bits forever (kills `pos -= 1` -> `+= 1` / `/= 1`, which would
        // advance past the marker and start reading the following bytes).
        let mut reader = BitReader::new(&[0xFF, 0xD9]);
        for _ in 0..24 {
            assert_eq!(reader.next_bit(), 1);
        }
        // A stuffed `FF 00` decodes to the data bits of `FF` then continues to the next byte.
        let mut reader = BitReader::new(&[0xFF, 0x00, 0x00]);
        for _ in 0..8 {
            assert_eq!(reader.next_bit(), 1); // the FF
        }
        for _ in 0..8 {
            assert_eq!(reader.next_bit(), 0); // the following 0x00
        }
    }

    /// Builds a valid 1x1x1 stream, then lets the caller mutate it.
    fn valid_stream() -> Vec<u8> {
        encode(&[12345u16], 1, 1, 1, 16)
    }

    #[test]
    fn decode_rejects_bad_header() {
        // Too short / no SOI.
        assert!(decode(&[]).is_err());
        assert!(decode(&[MARKER, 0x00]).is_err());
        // `< 2` -> `==`: a 1-byte `FF` must error, not index `data[1]` out of bounds.
        assert!(decode(&[MARKER]).is_err());
        // `< 2` -> `<=`: a bare SOI (len 2) must reach the marker loop and fail with "missing SOS",
        // not the header's "not a JPEG" message.
        match decode(&[MARKER, SOI]) {
            Err(Error::InvalidInput(m)) => assert!(m.contains("missing SOS"), "got {m:?}"),
            Err(other) => panic!("expected missing-SOS error, got {other:?}"),
            Ok(_) => panic!("expected missing-SOS error, got Ok"),
        }
        // Corrupting only the SOI's `FF` (byte 0) must fail: the `||` chain has to OR the
        // marker checks, not AND them (kills both `||` -> `&&` in the header guard).
        let mut s = valid_stream();
        s[0] = 0x00;
        assert!(decode(&s).is_err());
    }

    #[test]
    fn decode_skips_stray_bytes_and_unknown_segments() {
        // A stray non-marker byte right after SOI must be scanned over to find SOF3 (kills
        // `!= MARKER` -> `==`, and the scan filter `< data.len()` -> `==`/`>`).
        let s = valid_stream();
        let mut spliced = s[..2].to_vec();
        spliced.push(0xAA);
        spliced.extend_from_slice(&s[2..]);
        let decoded = decode(&spliced).expect("decode past stray byte");
        assert_eq!(decoded.samples, vec![12345u16]);

        // An unknown marker segment (APP0-like) must be skipped by its length (kills the `_`-arm
        // `pos += len` -> `-=` / `*=`).
        let mut spliced = s[..2].to_vec();
        spliced.extend_from_slice(&[MARKER, 0xE0, 0x00, 0x04, 0x00, 0x00]); // FF E0, len=4, 2 pad
        spliced.extend_from_slice(&s[2..]);
        let decoded = decode(&spliced).expect("decode past unknown segment");
        assert_eq!(decoded.samples, vec![12345u16]);
    }

    #[test]
    fn decode_rejects_eoi_before_sos() {
        // An EOI marker encountered before SOS must error (kills `delete match arm EOI`, which
        // would treat EOI as a skippable segment and then decode the spliced-in real markers).
        let s = valid_stream();
        let mut spliced = s[..2].to_vec();
        spliced.extend_from_slice(&[MARKER, EOI, 0x00, 0x04, 0x00, 0x00]); // FF D9, len=4, 2 pad
        spliced.extend_from_slice(&s[2..]);
        assert!(decode(&spliced).is_err());
    }

    #[test]
    fn decode_rejects_zero_dimensions() {
        // Patch the SOF3 width field (bytes 9..11 of a 1x1 stream) to zero. Each dimension is
        // independently rejected (kills both `|| ` -> `&&` in the zero-dimension guard).
        let mut s = valid_stream();
        s[9] = 0;
        s[10] = 0;
        assert!(decode(&s).is_err());
        // Likewise patch height (bytes 7..9) to zero.
        let mut s = valid_stream();
        s[7] = 0;
        s[8] = 0;
        assert!(decode(&s).is_err());
    }
}
