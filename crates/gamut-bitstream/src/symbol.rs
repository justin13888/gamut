//! AV1 multi-symbol arithmetic (range) encoder (AV1 §8.2, encoder side).
//!
//! The AV1 spec only defines the *decoder* (§8.2 "Parsing process for symbol decoder"). This is
//! the matching encoder: it produces a byte stream that the §8.2 decoder maps back to the symbols
//! that were encoded. The arithmetic mirrors the well-known `od_ec` range coder (the same one in
//! libaom / rav1e), which is purpose-built for this decoder.
//!
//! CDF convention (matches §8.2.6): a CDF for `N` symbols is a slice of `N` cumulative values in
//! `[0, 32768]`, strictly non-decreasing, with `cdf[N - 1] == 32768`. `cdf[i]` is the cumulative
//! probability (× 32768) of symbols `0..=i`. The adaptation counter the spec stores as a trailing
//! `cdf[N]` element is irrelevant here: this MVP runs with `disable_cdf_update = 1`, so CDFs are
//! static and never adapted. Adaptation is deferred to M1 (see `gamut-avif/STATUS.md`).
//!
//! The hermetic `SymbolDecoder` in this module's tests is a direct transcription of §8.2 and is
//! the oracle that proves the encoder correct without any external decoder.

/// Number of bits to reduce CDF precision during arithmetic coding (AV1 `EC_PROB_SHIFT`, §3).
const EC_PROB_SHIFT: u32 = 6;
/// Minimum probability assigned to each symbol during arithmetic coding (AV1 `EC_MIN_PROB`, §3).
const EC_MIN_PROB: u32 = 4;
/// CDFs are expressed on a 1 << 15 scale (AV1 §8.2.6: `cdf[N - 1] == 1 << 15`).
const CDF_PROB_TOP: u32 = 1 << 15;

/// Encoder for the AV1 symbol (range) coder.
///
/// Feed symbols with [`SymbolEncoder::encode_symbol`] (CDF-coded) and equiprobable bits with
/// [`SymbolEncoder::encode_literal`], then call [`SymbolEncoder::finish`] to flush and obtain the
/// coded bytes. Those bytes are exactly what a decoder consumes via `init_symbol(sz)` (AV1 §8.2.2)
/// where `sz` is the returned length.
#[derive(Debug, Clone)]
pub struct SymbolEncoder {
    /// Low end of the coding interval, kept wider than 16 bits so carries accumulate losslessly
    /// (resolved in [`SymbolEncoder::finish`]).
    low: u64,
    /// Current range, renormalised into `[1 << 15, 1 << 16)`.
    rng: u32,
    /// Bit counter; starts at `-9` so the first carry/byte crosses zero at the right moment.
    cnt: i32,
    /// Output bytes, each held as a `u16` so a pending carry lives in bit 8 until `finish`.
    precarry: Vec<u16>,
}

impl Default for SymbolEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl SymbolEncoder {
    /// Creates an encoder with the initial range state of AV1's symbol coder.
    #[must_use]
    pub fn new() -> Self {
        Self {
            low: 0,
            rng: CDF_PROB_TOP,
            cnt: -9,
            precarry: Vec::new(),
        }
    }

    /// Encodes `symbol` against a static cumulative `cdf` (`cdf.len()` symbols, `cdf[last] == 32768`).
    ///
    /// # Panics
    ///
    /// Debug builds assert `symbol < cdf.len()` and the CDF normalisation invariants.
    pub fn encode_symbol(&mut self, symbol: usize, cdf: &[u16]) {
        let nsyms = cdf.len();
        debug_assert!(symbol < nsyms);
        debug_assert_eq!(u32::from(cdf[nsyms - 1]), CDF_PROB_TOP);
        // `f(j) = (1 << 15) - cdf[j]` is the inverse-CDF term used by the §8.2.6 decoder; `fl`/`fh`
        // bracket the chosen symbol's sub-interval. For symbol 0, the upper bracket is the full top.
        let fl = if symbol > 0 {
            CDF_PROB_TOP - u32::from(cdf[symbol - 1])
        } else {
            CDF_PROB_TOP
        };
        let fh = CDF_PROB_TOP - u32::from(cdf[symbol]);
        self.encode_q15(fl, fh, symbol as u32, nsyms as u32);
    }

    /// Encodes the low `n` bits of `value` as equiprobable bits, most-significant bit first.
    ///
    /// This is the inverse of the decoder's `read_literal(n)` (AV1 §8.2.5), which itself calls
    /// `read_bool()` (§8.2.3) with the fixed CDF `{1 << 14, 1 << 15}`.
    pub fn encode_literal(&mut self, value: u32, n: u32) {
        const BOOL_CDF: [u16; 2] = [1 << 14, 1 << 15];
        for i in (0..n).rev() {
            self.encode_symbol(((value >> i) & 1) as usize, &BOOL_CDF);
        }
    }

    /// Core interval update for one symbol; `fl`/`fh` are the inverse-CDF brackets, `s` the symbol,
    /// `nsyms` the alphabet size. Mirrors `od_ec_encode_q15`, which inverts the §8.2.6 boundaries.
    fn encode_q15(&mut self, fl: u32, fh: u32, s: u32, nsyms: u32) {
        let mut low = self.low;
        let mut r = self.rng;
        debug_assert!(r >= CDF_PROB_TOP);
        let n = nsyms - 1;
        if fl < CDF_PROB_TOP {
            let u = (((r >> 8) * (fl >> EC_PROB_SHIFT)) >> (7 - EC_PROB_SHIFT))
                + EC_MIN_PROB * (n - (s - 1));
            let v =
                (((r >> 8) * (fh >> EC_PROB_SHIFT)) >> (7 - EC_PROB_SHIFT)) + EC_MIN_PROB * (n - s);
            debug_assert!(u <= r && v < u);
            low += u64::from(r - u);
            r = u - v;
        } else {
            // Symbol 0: the interval reaches the top, so `low` is unchanged.
            let v =
                (((r >> 8) * (fh >> EC_PROB_SHIFT)) >> (7 - EC_PROB_SHIFT)) + EC_MIN_PROB * (n - s);
            debug_assert!(v < r);
            r -= v;
        }
        self.normalize(low, r);
    }

    /// Renormalises `(low, rng)` back into `[1 << 15, 1 << 16)`, emitting completed bytes into
    /// `precarry`. Mirrors `od_ec_enc_normalize`.
    fn normalize(&mut self, mut low: u64, rng: u32) {
        // `d` = number of left shifts to bring `rng` to 16 bits. `rng` is in `[1, 0xFFFF]` here.
        let d = rng.leading_zeros() - 16;
        let mut c = self.cnt;
        let mut s = c + d as i32;
        if s >= 0 {
            c += 16;
            let mut m = (1u64 << c) - 1;
            if s >= 8 {
                self.precarry.push((low >> c) as u16);
                low &= m;
                c -= 8;
                m = (1u64 << c) - 1;
            }
            self.precarry.push((low >> c) as u16);
            s = c + d as i32 - 24;
            low &= m;
        }
        self.low = low << d;
        self.rng = rng << d;
        self.cnt = s;
    }

    /// Flushes the coder and returns the coded bytes. Mirrors `od_ec_enc_done`: it emits the
    /// minimum number of bits that decode correctly regardless of trailing padding, then resolves
    /// the buffered carries into a big-endian byte stream.
    #[must_use]
    pub fn finish(mut self) -> Vec<u8> {
        let l = self.low;
        let mut c = self.cnt;
        let mut s = 10 + c;
        let m: u64 = 0x3FFF;
        let mut e = ((l + m) & !m) | (m + 1);
        if s > 0 {
            let mut n = (1u64 << (c + 16)) - 1;
            loop {
                self.precarry.push((e >> (c + 16)) as u16);
                e &= n;
                s -= 8;
                c -= 8;
                n >>= 8;
                if s <= 0 {
                    break;
                }
            }
        }
        // Resolve carries from least- to most-significant byte (big-endian output).
        let mut out = vec![0u8; self.precarry.len()];
        let mut carry: u32 = 0;
        for i in (0..self.precarry.len()).rev() {
            let val = u32::from(self.precarry[i]) + carry;
            out[i] = (val & 0xff) as u8;
            carry = val >> 8;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Direct transcription of the AV1 §8.2 symbol decoder — the hermetic oracle for the encoder.
    struct SymbolDecoder<'a> {
        data: &'a [u8],
        bit_pos: usize,
        value: u32,
        range: u32,
        max_bits: i64,
    }

    impl<'a> SymbolDecoder<'a> {
        /// `f(n)` parsing process (AV1 §8.1): MSB-first, zero-padded past the end of `data`.
        fn read_f(&mut self, n: u32) -> u32 {
            let mut x = 0u32;
            for _ in 0..n {
                let idx = self.bit_pos >> 3;
                let bit = if idx < self.data.len() {
                    (self.data[idx] >> (7 - (self.bit_pos & 7))) & 1
                } else {
                    0
                };
                x = (x << 1) | u32::from(bit);
                self.bit_pos += 1;
            }
            x
        }

        /// `init_symbol(sz)` (AV1 §8.2.2).
        fn new(data: &'a [u8]) -> Self {
            let sz = data.len();
            let mut d = Self {
                data,
                bit_pos: 0,
                value: 0,
                range: 1 << 15,
                max_bits: 8 * sz as i64 - 15,
            };
            let num_bits = core::cmp::min(sz * 8, 15) as u32;
            let buf = d.read_f(num_bits);
            let padded = buf << (15 - num_bits);
            d.value = ((1 << 15) - 1) ^ padded;
            d
        }

        /// `read_symbol(cdf)` (AV1 §8.2.6); `cdf` is the cumulative form (no trailing count needed
        /// because adaptation is disabled).
        fn read_symbol(&mut self, cdf: &[u16]) -> usize {
            let n = cdf.len() as u32;
            let mut cur = self.range;
            let mut symbol: i64 = -1;
            let mut prev;
            loop {
                symbol += 1;
                prev = cur;
                let f = (1u32 << 15) - u32::from(cdf[symbol as usize]);
                cur = ((self.range >> 8) * (f >> EC_PROB_SHIFT)) >> (7 - EC_PROB_SHIFT);
                cur += EC_MIN_PROB * (n - symbol as u32 - 1);
                if self.value >= cur {
                    break;
                }
            }
            self.range = prev - cur;
            self.value -= cur;
            // Renormalisation (AV1 §8.2.6 ordered steps).
            let bits = 15 - (31 - self.range.leading_zeros());
            self.range <<= bits;
            let num_bits = core::cmp::min(i64::from(bits), self.max_bits.max(0)) as u32;
            let new_data = self.read_f(num_bits);
            let padded = new_data << (bits - num_bits);
            self.value = padded ^ (((self.value + 1) << bits) - 1);
            self.max_bits -= i64::from(bits);
            symbol as usize
        }

        fn read_literal(&mut self, n: u32) -> u32 {
            const BOOL_CDF: [u16; 2] = [1 << 14, 1 << 15];
            let mut x = 0;
            for _ in 0..n {
                x = (x << 1) | self.read_symbol(&BOOL_CDF) as u32;
            }
            x
        }
    }

    /// Small deterministic LCG so tests are reproducible without `rand`.
    struct Lcg(u64);
    impl Lcg {
        fn next_u32(&mut self) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 32) as u32
        }
        fn below(&mut self, bound: u32) -> u32 {
            self.next_u32() % bound
        }
    }

    /// Builds a random strictly-increasing cumulative CDF for `nsyms` symbols, `cdf[last] = 32768`.
    fn random_cdf(rng: &mut Lcg, nsyms: usize) -> Vec<u16> {
        // Pick `nsyms - 1` distinct breakpoints in 1..32768, sorted, then append 32768.
        let mut points = Vec::new();
        while points.len() < nsyms - 1 {
            let p = 1 + rng.below(32767) as u16;
            if !points.contains(&p) {
                points.push(p);
            }
        }
        points.sort_unstable();
        points.push(32768);
        points
    }

    #[test]
    fn empty_stream_roundtrips() {
        let enc = SymbolEncoder::new();
        let bytes = enc.finish();
        // Nothing to decode; just ensure init does not panic.
        let _ = SymbolDecoder::new(&bytes);
    }

    #[test]
    fn single_symbol_streams_roundtrip() {
        // Exhaustively exercise small alphabets with a skewed CDF and every symbol value.
        for nsyms in 2..=12usize {
            let mut cdf: Vec<u16> = (1..nsyms).map(|i| (i * 32768 / nsyms) as u16).collect();
            cdf.push(32768);
            for s in 0..nsyms {
                let mut enc = SymbolEncoder::new();
                enc.encode_symbol(s, &cdf);
                let bytes = enc.finish();
                let mut dec = SymbolDecoder::new(&bytes);
                assert_eq!(dec.read_symbol(&cdf), s, "nsyms={nsyms} s={s}");
            }
        }
    }

    #[test]
    fn long_random_symbol_stream_roundtrips() {
        let mut rng = Lcg(0x1234_5678_9abc_def0);
        // Pre-generate a mix of CDFs of varying sizes.
        let cdfs: Vec<Vec<u16>> = (2..=14).map(|n| random_cdf(&mut rng, n)).collect();
        let mut events = Vec::new();
        let mut enc = SymbolEncoder::new();
        for _ in 0..20_000 {
            let cdf = &cdfs[rng.below(cdfs.len() as u32) as usize];
            let s = rng.below(cdf.len() as u32) as usize;
            enc.encode_symbol(s, cdf);
            events.push((s, cdf.clone()));
        }
        let bytes = enc.finish();
        let mut dec = SymbolDecoder::new(&bytes);
        for (i, (s, cdf)) in events.iter().enumerate() {
            assert_eq!(dec.read_symbol(cdf), *s, "event {i}");
        }
    }

    #[test]
    fn literals_roundtrip() {
        let mut rng = Lcg(0xdead_beef_0bad_f00d);
        let mut enc = SymbolEncoder::new();
        let mut events = Vec::new();
        for _ in 0..5000 {
            let n = 1 + rng.below(16);
            let v = rng.next_u32() & ((1u32 << n) - 1);
            enc.encode_literal(v, n);
            events.push((v, n));
        }
        let bytes = enc.finish();
        let mut dec = SymbolDecoder::new(&bytes);
        for (v, n) in events {
            assert_eq!(dec.read_literal(n), v);
        }
    }

    #[test]
    fn mixed_symbols_and_literals_roundtrip() {
        let mut rng = Lcg(0x0f0f_0f0f_1234_9999);
        let cdf = random_cdf(&mut rng, 8);
        let mut enc = SymbolEncoder::new();
        let mut events: Vec<(bool, u32)> = Vec::new(); // (is_literal, payload)
        for _ in 0..8000 {
            if rng.next_u32() & 1 == 0 {
                let s = rng.below(cdf.len() as u32);
                enc.encode_symbol(s as usize, &cdf);
                events.push((false, s));
            } else {
                let v = rng.next_u32() & 0xff;
                enc.encode_literal(v, 8);
                events.push((true, v));
            }
        }
        let bytes = enc.finish();
        let mut dec = SymbolDecoder::new(&bytes);
        for (is_lit, payload) in events {
            if is_lit {
                assert_eq!(dec.read_literal(8), payload);
            } else {
                assert_eq!(dec.read_symbol(&cdf) as u32, payload);
            }
        }
    }
}
