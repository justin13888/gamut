//! The entropy-coded-segment bit writer: MSB-first bit packing with JPEG's byte stuffing
//! (§B.1.1.5) and 1-bit padding.
//!
//! JPEG entropy data is a stream of variable-length Huffman codes and magnitude bits packed
//! most-significant-bit first. Two rules make it *not* a plain bit writer, which is why
//! `gamut_bitstream::BitWriter` (also MSB-first, but it zero-pads on alignment and never stuffs) is
//! unsuitable and this small writer exists instead:
//!
//! - **Byte stuffing (§B.1.1.5, NOTE 2).** Any `0xFF` byte produced by the coder — including one
//!   created by the 1-bit padding below — is followed by a stuffed `0x00`, so an entropy byte can
//!   never be mistaken for a marker.
//! - **1-bit padding (§B.1.1.5, NOTE 1 / §F.1.2.3).** An entropy-coded segment is an integer number
//!   of bytes; the final partial byte is padded with **1** bits (not 0) before the following marker.
//!
//! The writer owns the output buffer so it can also drop in restart markers ([`BitWriter::restart`])
//! — which are markers, written un-stuffed, immediately after flushing the current segment's padding.

/// An MSB-first bit accumulator that writes JPEG entropy-coded data into an output buffer, applying
/// byte stuffing on every emitted byte.
pub struct BitWriter<'a> {
    out: &'a mut Vec<u8>,
    /// Bit accumulator; the `count` least-significant bits are pending output (MSB emitted first).
    acc: u64,
    /// Number of pending bits held in `acc`.
    count: u32,
}

impl<'a> BitWriter<'a> {
    /// Wraps `out`, appending entropy bytes to whatever it already holds.
    pub fn new(out: &'a mut Vec<u8>) -> Self {
        Self {
            out,
            acc: 0,
            count: 0,
        }
    }

    /// Appends the low `length` bits of `value` (MSB-first). `length` must be `0..=16`; a length of
    /// 0 is a no-op (used for the zero additional bits of a category-0 DC difference).
    pub fn write_bits(&mut self, value: u16, length: u8) {
        if length == 0 {
            return;
        }
        let length = u32::from(length);
        let masked = u64::from(value) & ((1u64 << length) - 1);
        // Compose with `+`: the shift just vacated `length` low zero bits and `masked < 2^length`,
        // so the addition fills exactly those bits (equivalent to `|` on disjoint operands).
        self.acc = (self.acc << length) + masked;
        self.count += length;
        while self.count >= 8 {
            self.count -= 8;
            let byte = (self.acc >> self.count) as u8;
            self.emit(byte);
        }
        // Keep only the still-pending low bits so `acc` never grows without bound.
        self.acc &= (1u64 << self.count) - 1;
    }

    /// Emits one entropy byte, stuffing a `0x00` after a `0xFF` (§B.1.1.5).
    fn emit(&mut self, byte: u8) {
        self.out.push(byte);
        if byte == 0xFF {
            self.out.push(0x00);
        }
    }

    /// Pads the final partial byte with 1-bits and emits it (§B.1.1.5 NOTE 1). Idempotent when the
    /// stream is already byte-aligned. Called before every marker (restart or end-of-scan).
    pub fn flush(&mut self) {
        if self.count > 0 {
            let pad = 8 - self.count;
            // Shift in `pad` one-bits, then emit the completed byte (with stuffing). Composed with
            // `+`: `2^pad − 1` fills exactly the `pad` low zero bits the shift vacated.
            self.acc = (self.acc << pad) + ((1u64 << pad) - 1);
            let byte = self.acc as u8;
            self.emit(byte);
            self.acc = 0;
            self.count = 0;
        }
    }

    /// Flushes the current segment (1-bit padding) and writes a restart marker `RSTm` for
    /// `m` in `0..=7`, un-stuffed (§B.2.1). The DC predictors are reset by the caller.
    pub fn restart(&mut self, m: u8) {
        self.flush();
        self.out.push(0xFF);
        self.out.push(crate::marker::code::RST0 + (m & 7));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packs_bits_msb_first() {
        // 0b101 (len 3) then 0b01 (len 2) then pad → 1 0 1 0 1 | 111 = 0b10101111 = 0xAF.
        let mut out = Vec::new();
        let mut w = BitWriter::new(&mut out);
        w.write_bits(0b101, 3);
        w.write_bits(0b01, 2);
        w.flush();
        assert_eq!(out, vec![0xAF]);
    }

    #[test]
    fn one_bit_padding_uses_ones_not_zeros() {
        // A single 0-bit then flush must pad with 1s → 0b0111_1111 = 0x7F, proving the pad is 1-bits.
        let mut out = Vec::new();
        let mut w = BitWriter::new(&mut out);
        w.write_bits(0, 1);
        w.flush();
        assert_eq!(out, vec![0x7F]);
    }

    #[test]
    fn flush_is_a_noop_when_already_aligned() {
        // Exactly 8 bits emit one byte; a following flush must add nothing (no stray padding byte).
        let mut out = Vec::new();
        let mut w = BitWriter::new(&mut out);
        w.write_bits(0xA5, 8);
        w.flush();
        w.flush();
        assert_eq!(out, vec![0xA5]);
    }

    #[test]
    fn stuffs_zero_after_ff() {
        // A full 0xFF byte must be followed by a stuffed 0x00 so it can't look like a marker.
        let mut out = Vec::new();
        let mut w = BitWriter::new(&mut out);
        w.write_bits(0xFF, 8);
        assert_eq!(out, vec![0xFF, 0x00]);
    }

    #[test]
    fn stuffs_ff_created_by_padding() {
        // Eight 1-bits' worth via a 0-length-forcing case: write 0x7F (7 bits all 1) then pad 1 bit
        // → 0xFF, which must also be stuffed. Here write 7 ones then flush.
        let mut out = Vec::new();
        let mut w = BitWriter::new(&mut out);
        w.write_bits(0b111_1111, 7);
        w.flush(); // pads one 1-bit → 0xFF → stuffed
        assert_eq!(out, vec![0xFF, 0x00]);
    }

    #[test]
    fn write_bits_zero_length_is_noop() {
        // A zero-length write adds no bits: the result is identical to omitting it. Here 0b101 (3
        // bits) then flush pads five 1-bits → 0b1011_1111 = 0xBF.
        let mut out = Vec::new();
        let mut w = BitWriter::new(&mut out);
        w.write_bits(0, 0);
        w.write_bits(0b101, 3);
        w.write_bits(0, 0);
        w.flush();
        assert_eq!(out, vec![0xBF]);
    }

    #[test]
    fn sixteen_bit_code_spans_two_bytes() {
        // A full 16-bit code (the ZRL length) must serialize as its two big-endian bytes.
        let mut out = Vec::new();
        let mut w = BitWriter::new(&mut out);
        w.write_bits(0b1111_1111_1000_0010, 16);
        w.flush();
        assert_eq!(out, vec![0xFF, 0x00, 0x82]); // 0xFF stuffed, then 0x82
    }

    #[test]
    fn restart_flushes_then_writes_unstuffed_marker() {
        // Pending bits are padded (1s), then RST3 is written raw as 0xFF 0xD3 (NOT stuffed).
        let mut out = Vec::new();
        let mut w = BitWriter::new(&mut out);
        w.write_bits(0, 1); // one 0-bit pending
        w.restart(3);
        assert_eq!(out, vec![0x7F, 0xFF, 0xD3]);
    }

    #[test]
    fn restart_marker_cycles_low_three_bits() {
        let mut out = Vec::new();
        let mut w = BitWriter::new(&mut out);
        w.restart(8); // 8 & 7 == 0 → RST0
        assert_eq!(out, vec![0xFF, 0xD0]);
    }
}
