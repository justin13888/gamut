//! VP8L canonical prefix (Huffman) codes (RFC 9649 §3.7).
//!
//! VP8L entropy-codes symbols with canonical prefix codes built from per-symbol code lengths. A
//! prefix-code group bundles five codes (green+length+cache, red, blue, alpha, distance), and meta
//! prefix codes select a group per block via an entropy image (§3.7.1-§3.7.3).
//!
//! # Bit order
//!
//! VP8L canonical codes are assigned in the usual increasing-by-(length, symbol) manner — the same
//! convention as DEFLATE — but written into an **LSB-first** stream. The encoder emits each code
//! *bit-reversed* to its length (see [`reverse_bits`]) so that the first bit on the wire is the
//! code's most-significant bit; the decoder ([`PrefixCode::read_symbol`]) reads bit by bit,
//! rebuilding the code MSB-first (the canonical "puff" decode), so no reversal is needed on read.
//!
//! # Single-symbol codes
//!
//! A code with a single used symbol is a complete tree that **consumes no bits** (RFC 9649 §3.7.2):
//! the symbol is implicit. Both [`PrefixEncoder::write_symbol`] (writes nothing) and
//! [`PrefixCode::read_symbol`] (returns the symbol without reading) honor this, so they stay in
//! lock-step. An empty alphabet is coded as a single symbol `0`.

use gamut_core::{Error, Result};

use crate::vp8l::bit_io::{BitReader, BitWriter};

/// Maximum prefix-code length in bits (RFC 9649 §3.7.2).
pub const MAX_CODE_LENGTH: usize = 15;
/// Number of literal symbols per channel (a full 8-bit byte).
pub const NUM_LITERAL_CODES: usize = 256;
/// Number of LZ77 length prefix codes packed into the green alphabet (§5.2.2).
pub const NUM_LENGTH_CODES: usize = 24;
/// Number of distance prefix codes (§5.2.2).
pub const NUM_DISTANCE_CODES: usize = 40;
/// Number of code-length code symbols (literals 0..=15 plus repeat codes 16/17/18) (§3.7.2).
pub const CODE_LENGTH_CODES: usize = 19;

/// The order in which code-length code lengths appear on the wire (RFC 9649 §3.7.2).
pub const CODE_LENGTH_CODE_ORDER: [usize; CODE_LENGTH_CODES] = [
    17, 18, 0, 1, 2, 3, 4, 5, 16, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
];

/// Default code length assumed by repeat-code 16 before any nonzero length is seen (§3.7.2).
const DEFAULT_CODE_LENGTH: u8 = 8;

/// Size of the green/length/cache alphabet for a given color-cache size (0 if the cache is off).
#[must_use]
pub fn green_alphabet_size(color_cache_size: usize) -> usize {
    NUM_LITERAL_CODES + NUM_LENGTH_CODES + color_cache_size
}

/// Reverses the low `num_bits` of `value` (used to emit canonical codes MSB-first into the
/// LSB-first stream).
#[must_use]
pub fn reverse_bits(value: u16, num_bits: u8) -> u16 {
    let mut v = value;
    let mut r = 0u16;
    for _ in 0..num_bits {
        r = (r << 1) | (v & 1);
        v >>= 1;
    }
    r
}

/// A canonical prefix (Huffman) decoder built from per-symbol code lengths (RFC 9649 §3.7.2).
///
/// Decoding uses the classic canonical algorithm (no large lookup table): bits are read one at a
/// time and accumulated MSB-first until they identify a symbol. A single-symbol code returns its
/// symbol without consuming any bits.
#[derive(Debug, Clone)]
pub struct PrefixCode {
    /// `counts[len]` = number of symbols coded with length `len`.
    counts: [u16; MAX_CODE_LENGTH + 1],
    /// Symbols sorted by `(length, symbol)`.
    symbols: Vec<u16>,
    /// Set for a single-symbol code (consumes 0 bits, always returns this symbol).
    single: Option<u16>,
}

impl PrefixCode {
    /// Builds a decoder from `code_lengths` (one entry per symbol; `0` = unused).
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if a length exceeds [`MAX_CODE_LENGTH`], if no symbol is
    /// used, or if the lengths do not form a complete tree (the single-symbol leaf is the one
    /// permitted incomplete tree, per §3.7.2).
    pub fn from_code_lengths(code_lengths: &[u8]) -> Result<Self> {
        let mut counts = [0u16; MAX_CODE_LENGTH + 1];
        let mut n_used = 0usize;
        let mut last_used = 0u16;
        for (sym, &len) in code_lengths.iter().enumerate() {
            if len as usize > MAX_CODE_LENGTH {
                return Err(Error::InvalidInput("VP8L: prefix code length too large"));
            }
            if len > 0 {
                counts[len as usize] += 1;
                n_used += 1;
                last_used = sym as u16;
            }
        }
        if n_used == 0 {
            return Err(Error::InvalidInput("VP8L: empty prefix code"));
        }
        if n_used == 1 {
            return Ok(Self {
                counts,
                symbols: Vec::new(),
                single: Some(last_used),
            });
        }
        // Completeness check (Kraft equality), over-subscription detected as a negative remainder.
        let mut left = 1i32;
        for &count in counts.iter().take(MAX_CODE_LENGTH + 1).skip(1) {
            left <<= 1;
            left -= i32::from(count);
            if left < 0 {
                return Err(Error::InvalidInput("VP8L: over-subscribed prefix code"));
            }
        }
        if left != 0 {
            return Err(Error::InvalidInput("VP8L: incomplete prefix code"));
        }
        // Sort symbols by (length, symbol) into a flat table.
        let mut offsets = [0usize; MAX_CODE_LENGTH + 2];
        for len in 1..=MAX_CODE_LENGTH {
            offsets[len + 1] = offsets[len] + usize::from(counts[len]);
        }
        let mut symbols = vec![0u16; n_used];
        for (sym, &len) in code_lengths.iter().enumerate() {
            if len > 0 {
                let slot = &mut offsets[len as usize];
                symbols[*slot] = sym as u16;
                *slot += 1;
            }
        }
        Ok(Self {
            counts,
            symbols,
            single: None,
        })
    }

    /// Reads one symbol from `r`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] on truncation or if the bits do not match any code.
    pub fn read_symbol(&self, r: &mut BitReader<'_>) -> Result<u16> {
        if let Some(sym) = self.single {
            return Ok(sym);
        }
        let mut code: i32 = 0;
        let mut first: i32 = 0;
        let mut index: usize = 0;
        for len in 1..=MAX_CODE_LENGTH {
            code |= r.read_bit()? as i32;
            let count = i32::from(self.counts[len]);
            if code - first < count {
                let pos = index + (code - first) as usize;
                return self
                    .symbols
                    .get(pos)
                    .copied()
                    .ok_or(Error::InvalidInput("VP8L: prefix code index out of range"));
            }
            index += count as usize;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(Error::InvalidInput("VP8L: invalid prefix code"))
    }
}

/// Reads a single prefix code's lengths from the bitstream (simple or normal variant) and builds it
/// (RFC 9649 §3.7.2). `alphabet_size` bounds the symbols and `max_symbol`.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] on any malformed code-length coding or truncation.
pub fn read_prefix_code(r: &mut BitReader<'_>, alphabet_size: usize) -> Result<PrefixCode> {
    if r.read_bit()? == 1 {
        read_simple_prefix_code(r, alphabet_size)
    } else {
        read_normal_prefix_code(r, alphabet_size)
    }
}

/// Reads the *simple code length code* variant: 1 or 2 symbols, each with code length 1 (§3.7.2).
fn read_simple_prefix_code(r: &mut BitReader<'_>, alphabet_size: usize) -> Result<PrefixCode> {
    let num_symbols = r.read_bit()? + 1; // 1 or 2
    let is_first_8bits = r.read_bit()?;
    let mut lengths = vec![0u8; alphabet_size];
    let symbol0 = r.read_bits(1 + 7 * is_first_8bits)? as usize;
    if symbol0 >= alphabet_size {
        return Err(Error::InvalidInput(
            "VP8L: simple prefix symbol out of range",
        ));
    }
    lengths[symbol0] = 1;
    if num_symbols == 2 {
        let symbol1 = r.read_bits(8)? as usize;
        if symbol1 >= alphabet_size {
            return Err(Error::InvalidInput(
                "VP8L: simple prefix symbol out of range",
            ));
        }
        lengths[symbol1] = 1;
    }
    PrefixCode::from_code_lengths(&lengths)
}

/// Reads the *normal code length code* variant (§3.7.2): a meta code over `code_length_code_lengths`
/// drives literal lengths plus the repeat codes 16/17/18.
fn read_normal_prefix_code(r: &mut BitReader<'_>, alphabet_size: usize) -> Result<PrefixCode> {
    let num_code_lengths = 4 + r.read_bits(4)? as usize;
    if num_code_lengths > CODE_LENGTH_CODES {
        return Err(Error::InvalidInput("VP8L: too many code-length codes"));
    }
    let mut cl_lengths = [0u8; CODE_LENGTH_CODES];
    for &order in CODE_LENGTH_CODE_ORDER.iter().take(num_code_lengths) {
        cl_lengths[order] = r.read_bits(3)? as u8;
    }
    let cl_code = PrefixCode::from_code_lengths(&cl_lengths)?;

    let mut max_symbol = if r.read_bit()? != 0 {
        let length_nbits = 2 + 2 * r.read_bits(3)?;
        2 + r.read_bits(length_nbits)? as usize
    } else {
        alphabet_size
    };
    if max_symbol > alphabet_size {
        return Err(Error::InvalidInput("VP8L: max_symbol exceeds alphabet"));
    }

    let mut lengths = vec![0u8; alphabet_size];
    let mut prev_len = DEFAULT_CODE_LENGTH;
    let mut symbol = 0usize;
    while symbol < alphabet_size {
        if max_symbol == 0 {
            break;
        }
        max_symbol -= 1;
        let code = cl_code.read_symbol(r)?;
        if code < 16 {
            lengths[symbol] = code as u8;
            symbol += 1;
            if code != 0 {
                prev_len = code as u8;
            }
        } else {
            let (extra_bits, repeat_offset, value) = match code {
                16 => (2u32, 3usize, prev_len),
                17 => (3, 3, 0),
                18 => (7, 11, 0),
                _ => return Err(Error::InvalidInput("VP8L: invalid code-length symbol")),
            };
            let repeat = repeat_offset + r.read_bits(extra_bits)? as usize;
            if symbol + repeat > alphabet_size {
                return Err(Error::InvalidInput(
                    "VP8L: code-length repeat overruns alphabet",
                ));
            }
            for _ in 0..repeat {
                lengths[symbol] = value;
                symbol += 1;
            }
        }
    }
    PrefixCode::from_code_lengths(&lengths)
}

/// The five canonical codes used to decode a pixel (RFC 9649 §3.7.1).
#[derive(Debug, Clone)]
pub struct PrefixCodeGroup {
    /// Green channel, LZ77 lengths, and color-cache indices.
    pub green: PrefixCode,
    /// Red channel.
    pub red: PrefixCode,
    /// Blue channel.
    pub blue: PrefixCode,
    /// Alpha channel.
    pub alpha: PrefixCode,
    /// LZ77 distance codes.
    pub distance: PrefixCode,
}

/// Reads a [`PrefixCodeGroup`] (five codes in bitstream order); the green alphabet grows by
/// `color_cache_size` (0 when the cache is off).
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] on any malformed code or truncation.
pub fn read_prefix_code_group(
    r: &mut BitReader<'_>,
    color_cache_size: usize,
) -> Result<PrefixCodeGroup> {
    Ok(PrefixCodeGroup {
        green: read_prefix_code(r, green_alphabet_size(color_cache_size))?,
        red: read_prefix_code(r, NUM_LITERAL_CODES)?,
        blue: read_prefix_code(r, NUM_LITERAL_CODES)?,
        alpha: read_prefix_code(r, NUM_LITERAL_CODES)?,
        distance: read_prefix_code(r, NUM_DISTANCE_CODES)?,
    })
}

// --- Encoder side ---------------------------------------------------------------------------------

/// Derives the canonical (bit-reversed, ready-to-emit) codes for each symbol from its `lengths`.
///
/// Each returned code is reversed to its length so it can be written LSB-first with
/// [`BitWriter::write_bits`]; unused symbols (length 0) get code 0.
#[must_use]
pub fn canonical_codes(lengths: &[u8]) -> Vec<u16> {
    let mut bl_count = [0u32; MAX_CODE_LENGTH + 1];
    for &len in lengths {
        if len > 0 && (len as usize) <= MAX_CODE_LENGTH {
            bl_count[len as usize] += 1;
        }
    }
    let mut next_code = [0u32; MAX_CODE_LENGTH + 1];
    let mut code = 0u32;
    for bits in 1..=MAX_CODE_LENGTH {
        code = (code + bl_count[bits - 1]) << 1;
        next_code[bits] = code;
    }
    let mut codes = vec![0u16; lengths.len()];
    for (sym, &len) in lengths.iter().enumerate() {
        if len > 0 && (len as usize) <= MAX_CODE_LENGTH {
            let c = next_code[len as usize];
            next_code[len as usize] += 1;
            codes[sym] = reverse_bits(c as u16, len);
        }
    }
    codes
}

/// Builds length-limited (`<= max_len`) canonical Huffman code lengths from a symbol `histogram`.
///
/// Returns one length per symbol (`0` = unused). An empty histogram yields all-zero lengths (the
/// caller codes that as a single symbol `0`); a single nonzero symbol gets length 1. To bound the
/// maximum length, the histogram counts are raised toward a common floor and the tree rebuilt until
/// it fits (libwebp's approach) — this yields *a* valid code, not necessarily the optimal one
/// (density tuning is deferred to issue #31).
#[must_use]
pub fn build_length_limited_lengths(histogram: &[u32], max_len: u8) -> Vec<u8> {
    let n = histogram.len();
    let used = histogram.iter().filter(|&&h| h > 0).count();
    if used == 0 {
        return vec![0u8; n];
    }
    if used == 1 {
        let mut lengths = vec![0u8; n];
        if let Some(sym) = (0..n).find(|&i| histogram[i] > 0) {
            lengths[sym] = 1;
        }
        return lengths;
    }
    let mut count_min = 1u32;
    loop {
        let depths = huffman_pass(histogram, count_min);
        let max_depth = depths.iter().copied().max().unwrap_or(0);
        if max_depth <= u32::from(max_len) {
            return depths.iter().map(|&d| d as u8).collect();
        }
        count_min = count_min.saturating_mul(2);
    }
}

/// One Huffman construction pass; returns per-symbol depths (code lengths) with each present
/// symbol's weight floored to `count_min` (raising the floor flattens the tree, capping its depth).
fn huffman_pass(histogram: &[u32], count_min: u32) -> Vec<u32> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;

    /// A node in the Huffman tree (`sym >= 0` marks a leaf).
    struct Node {
        left: i32,
        right: i32,
        sym: i32,
    }

    let n = histogram.len();
    let mut lengths = vec![0u32; n];
    let mut nodes: Vec<Node> = Vec::new();
    // Min-heap keyed by (weight, tie-break index) for deterministic output.
    let mut heap: BinaryHeap<Reverse<(u64, usize)>> = BinaryHeap::new();
    for (sym, &count) in histogram.iter().enumerate() {
        if count > 0 {
            let idx = nodes.len();
            nodes.push(Node {
                left: -1,
                right: -1,
                sym: sym as i32,
            });
            heap.push(Reverse((u64::from(count.max(count_min)), idx)));
        }
    }
    if nodes.len() == 1 {
        if let Some(sym) = (0..n).find(|&i| histogram[i] > 0) {
            lengths[sym] = 1;
        }
        return lengths;
    }
    while heap.len() > 1 {
        let (Some(Reverse((wa, a))), Some(Reverse((wb, b)))) = (heap.pop(), heap.pop()) else {
            break;
        };
        let idx = nodes.len();
        nodes.push(Node {
            left: a as i32,
            right: b as i32,
            sym: -1,
        });
        heap.push(Reverse((wa + wb, idx)));
    }
    let Some(Reverse((_, root))) = heap.pop() else {
        return lengths;
    };
    // Assign depths with an explicit stack (the tree can be up to `used` deep).
    let mut stack = vec![(root, 0u32)];
    while let Some((idx, depth)) = stack.pop() {
        let Some(node) = nodes.get(idx) else { continue };
        if node.sym >= 0 {
            if let Some(slot) = lengths.get_mut(node.sym as usize) {
                *slot = depth;
            }
        } else {
            stack.push((node.left as usize, depth + 1));
            stack.push((node.right as usize, depth + 1));
        }
    }
    lengths
}

/// An encoder-side prefix code: per-symbol emit patterns + lengths, with the single-symbol
/// (0-bit) special case tracked so emission stays in lock-step with [`PrefixCode::read_symbol`].
#[derive(Debug, Clone)]
pub struct PrefixEncoder {
    lengths: Vec<u8>,
    codes: Vec<u16>,
    single: bool,
}

impl PrefixEncoder {
    /// Builds an encoder from per-symbol `lengths`.
    #[must_use]
    pub fn from_lengths(lengths: &[u8]) -> Self {
        let codes = canonical_codes(lengths);
        let single = lengths.iter().filter(|&&l| l > 0).count() <= 1;
        Self {
            lengths: lengths.to_vec(),
            codes,
            single,
        }
    }

    /// Per-symbol code lengths (one entry per symbol; `0` = unused).
    #[must_use]
    pub fn lengths(&self) -> &[u8] {
        &self.lengths
    }

    /// Writes `symbol` to `w`. A single-symbol code writes nothing (0 bits).
    pub fn write_symbol(&self, w: &mut BitWriter, symbol: usize) {
        if self.single {
            return;
        }
        if let (Some(&code), Some(&len)) = (self.codes.get(symbol), self.lengths.get(symbol)) {
            w.write_bits(u32::from(code), u32::from(len));
        }
    }
}

/// Writes a prefix code described by `lengths` using the *normal code length code* (RFC 9649
/// §3.7.2).
///
/// The code lengths are themselves prefix-coded; for simplicity each length is emitted literally
/// (no 16/17/18 run compression — that density win is deferred to issue #31), `max_symbol` is left
/// at the alphabet default, and the meta code is itself length-limited to the 3-bit field range.
pub fn write_normal_prefix_code(w: &mut BitWriter, lengths: &[u8]) {
    w.write_bits(0, 1); // 0 = normal (not simple) code length code.

    // Histogram the literal code-length symbols (only values 0..=15 occur in this literal scheme).
    let mut cl_hist = [0u32; CODE_LENGTH_CODES];
    for &len in lengths {
        if (len as usize) < CODE_LENGTH_CODES {
            cl_hist[len as usize] += 1;
        }
    }
    // The meta code's lengths are emitted in 3-bit fields, so they must fit in 7 bits.
    let cl_lengths = build_length_limited_lengths(&cl_hist, 7);
    let cl_encoder = PrefixEncoder::from_lengths(&cl_lengths);

    // Emit the meta code lengths in CODE_LENGTH_CODE_ORDER, trimming trailing zeros (min 4).
    let mut num_code_lengths = CODE_LENGTH_CODES;
    while num_code_lengths > 4 && cl_lengths[CODE_LENGTH_CODE_ORDER[num_code_lengths - 1]] == 0 {
        num_code_lengths -= 1;
    }
    w.write_bits((num_code_lengths - 4) as u32, 4);
    for &order in CODE_LENGTH_CODE_ORDER.iter().take(num_code_lengths) {
        w.write_bits(u32::from(cl_lengths[order]), 3);
    }

    w.write_bits(0, 1); // max_symbol uses the alphabet default.
    for &len in lengths {
        cl_encoder.write_symbol(w, len as usize);
    }
}

/// Writes a prefix code for 1 or 2 symbols using the *simple code length code* (RFC 9649 §3.7.2).
/// Each listed symbol is given code length 1. `symbols` must hold 1 or 2 entries; extra entries are
/// ignored.
pub fn write_simple_prefix_code(w: &mut BitWriter, symbols: &[u16]) {
    w.write_bits(1, 1); // 1 = simple code length code.
    let num_symbols = symbols.len().clamp(1, 2);
    w.write_bits((num_symbols - 1) as u32, 1);
    let symbol0 = symbols.first().copied().unwrap_or(0);
    let is_first_8bits = u32::from(symbol0 > 1);
    w.write_bits(is_first_8bits, 1);
    w.write_bits(u32::from(symbol0), 1 + 7 * is_first_8bits);
    if num_symbols == 2 {
        let symbol1 = symbols.get(1).copied().unwrap_or(0);
        w.write_bits(u32::from(symbol1), 8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_bits_matches_manual() {
        assert_eq!(reverse_bits(0b1, 1), 0b1);
        assert_eq!(reverse_bits(0b10, 2), 0b01);
        assert_eq!(reverse_bits(0b1011, 4), 0b1101);
        assert_eq!(reverse_bits(0b0000_0001, 8), 0b1000_0000);
        // Reversing twice is the identity for the given width.
        for v in 0u16..256 {
            assert_eq!(reverse_bits(reverse_bits(v, 8), 8), v);
        }
    }

    /// Round-trips a symbol stream through an encoder built from a histogram and a decoder built
    /// from the same lengths, exercising `build_length_limited_lengths` + `canonical_codes`.
    fn assert_code_round_trips(histogram: &[u32], stream: &[usize], max_len: u8) {
        let lengths = build_length_limited_lengths(histogram, max_len);
        assert!(lengths.iter().all(|&l| l <= max_len), "length exceeds cap");
        let encoder = PrefixEncoder::from_lengths(&lengths);
        let mut w = BitWriter::new();
        for &s in stream {
            encoder.write_symbol(&mut w, s);
        }
        let bytes = w.finish();
        let decoder = PrefixCode::from_code_lengths(&lengths).expect("valid lengths");
        let mut r = BitReader::new(&bytes);
        for &s in stream {
            assert_eq!(decoder.read_symbol(&mut r).unwrap() as usize, s);
        }
    }

    #[test]
    fn round_trips_varied_histograms() {
        // Uniform, skewed, two-symbol, and a single-symbol alphabet.
        let uniform: Vec<u32> = vec![1; 16];
        assert_code_round_trips(&uniform, &[0, 5, 15, 3, 8, 8, 0], 15);

        let mut skewed = vec![1u32; 32];
        skewed[7] = 10_000;
        skewed[19] = 2_000;
        assert_code_round_trips(&skewed, &[7, 7, 19, 0, 31, 7], 15);

        let mut two = vec![0u32; 256];
        two[10] = 5;
        two[200] = 9;
        assert_code_round_trips(&two, &[10, 200, 10, 10, 200], 15);

        let mut single = vec![0u32; 40];
        single[12] = 99;
        // Single-symbol code consumes no bits, so the stream decodes regardless of length.
        assert_code_round_trips(&single, &[12, 12, 12], 15);
    }

    #[test]
    fn forces_and_caps_15_bit_lengths() {
        // A Fibonacci-like distribution drives natural Huffman lengths well past 15; the limiter
        // must still cap them.
        let mut hist = vec![0u32; 64];
        let (mut a, mut b) = (1u32, 1u32);
        for h in hist.iter_mut() {
            *h = a;
            let next = a.saturating_add(b);
            a = b;
            b = next;
        }
        let lengths = build_length_limited_lengths(&hist, 15);
        assert!(lengths.iter().all(|&l| l <= 15));
        // Still a usable, complete code.
        let stream: Vec<usize> = (0..64).collect();
        assert_code_round_trips(&hist, &stream, 15);
    }

    #[test]
    fn normal_code_length_coding_round_trips() {
        // Build a code, serialize it with write_normal_prefix_code, read it back, and confirm the
        // reconstructed decoder agrees on a symbol stream.
        let mut hist = vec![0u32; 256];
        for (i, h) in hist.iter_mut().enumerate() {
            *h = (i as u32 % 7) + 1;
        }
        let lengths = build_length_limited_lengths(&hist, 15);
        let encoder = PrefixEncoder::from_lengths(&lengths);

        let stream: Vec<usize> = vec![0, 1, 2, 100, 255, 17, 42, 42, 7];
        let mut w = BitWriter::new();
        write_normal_prefix_code(&mut w, &lengths);
        for &s in &stream {
            encoder.write_symbol(&mut w, s);
        }
        let bytes = w.finish();

        let mut r = BitReader::new(&bytes);
        let decoder = read_prefix_code(&mut r, 256).expect("valid code description");
        for &s in &stream {
            assert_eq!(decoder.read_symbol(&mut r).unwrap() as usize, s);
        }
    }

    #[test]
    fn simple_code_length_coding_round_trips() {
        for symbols in [
            vec![0u16],
            vec![1u16],
            vec![5u16],
            vec![3u16, 200],
            vec![0u16, 1],
        ] {
            let mut lengths = vec![0u8; 256];
            for &s in &symbols {
                lengths[s as usize] = 1;
            }
            let encoder = PrefixEncoder::from_lengths(&lengths);
            let stream: Vec<usize> = symbols.iter().map(|&s| s as usize).collect();

            let mut w = BitWriter::new();
            write_simple_prefix_code(&mut w, &symbols);
            for &s in &stream {
                encoder.write_symbol(&mut w, s);
            }
            let bytes = w.finish();

            let mut r = BitReader::new(&bytes);
            let decoder = read_prefix_code(&mut r, 256).expect("valid simple code");
            for &s in &stream {
                assert_eq!(decoder.read_symbol(&mut r).unwrap() as usize, s);
            }
        }
    }

    #[test]
    fn rejects_malformed_lengths() {
        // Over-subscribed: three length-1 codes (Kraft sum > 1).
        assert!(matches!(
            PrefixCode::from_code_lengths(&[1, 1, 1]),
            Err(Error::InvalidInput(_))
        ));
        // Incomplete: a length-1 and a length-2 code leave the tree under-filled.
        assert!(matches!(
            PrefixCode::from_code_lengths(&[1, 2]),
            Err(Error::InvalidInput(_))
        ));
        // Length beyond the 15-bit cap.
        assert!(matches!(
            PrefixCode::from_code_lengths(&[16, 0]),
            Err(Error::InvalidInput(_))
        ));
        // Empty alphabet.
        assert!(matches!(
            PrefixCode::from_code_lengths(&[0, 0, 0]),
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn single_symbol_consumes_no_bits() {
        let code = PrefixCode::from_code_lengths(&[0, 0, 3, 0]).expect("single leaf");
        let mut r = BitReader::new(&[]); // no data at all
        assert_eq!(code.read_symbol(&mut r).unwrap(), 2);
        assert_eq!(code.read_symbol(&mut r).unwrap(), 2);
    }

    #[test]
    fn green_alphabet_size_includes_cache() {
        assert_eq!(green_alphabet_size(0), 280);
        assert_eq!(green_alphabet_size(1024), 280 + 1024);
    }

    #[test]
    fn reads_prefix_code_group() {
        // Emit five trivial single-symbol codes (each consumes no data) and read them as a group.
        let mut w = BitWriter::new();
        for _ in 0..5 {
            write_simple_prefix_code(&mut w, &[0]);
        }
        let bytes = w.finish();
        let mut r = BitReader::new(&bytes);
        let group = read_prefix_code_group(&mut r, 0).expect("group");
        let mut rr = BitReader::new(&[]);
        assert_eq!(group.green.read_symbol(&mut rr).unwrap(), 0);
        assert_eq!(group.distance.read_symbol(&mut rr).unwrap(), 0);
    }
}
