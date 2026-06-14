//! An **LSB-first** bit writer for DEFLATE (RFC 1951 §3.1.1).
//!
//! DEFLATE packs data elements into bytes starting from the least-significant bit. Fixed-width
//! fields (block header bits, extra bits, `LEN`/`NLEN`, `HLIT`/`HDIST`/`HCLEN`) are written LSB-first
//! directly. Huffman codes are instead packed *most-significant-bit-of-the-code first*; the
//! canonical-code builder handles that by pre-reversing each code's bits, so codes too are emitted
//! through [`BitWriter::write_bits`] with no special case here.
//!
//! Ported from the VP8L LSB-first writer in `gamut-webp` (both formats share this bit order); a
//! future refactor may graduate a shared writer into `gamut-bitstream`.

/// The widest single field DEFLATE writes at once is a 15-bit Huffman code or a 13-bit extra-bits
/// field; the cap is set to 32 to leave headroom. A wider request is a caller bug and writes nothing.
const MAX_BITS_PER_OP: u32 = 32;

/// An LSB-first bit writer that accumulates bits low-to-high and flushes a byte at a time. Infallible
/// (it grows a `Vec`).
#[derive(Debug, Clone)]
pub(crate) struct BitWriter {
    /// Completed bytes.
    buf: Vec<u8>,
    /// Pending bits, right-aligned (the next bit to emit is bit 0).
    acc: u64,
    /// Number of valid low bits in `acc` (always `< 8` between operations).
    bits_in_acc: u32,
}

impl BitWriter {
    /// Creates an empty writer.
    pub(crate) fn new() -> Self {
        Self {
            buf: Vec::new(),
            acc: 0,
            bits_in_acc: 0,
        }
    }

    /// Appends the low `n` bits (`0..=32`) of `value`, LSB-first. Bits above bit `n-1` are ignored;
    /// `n == 0` or `n > 32` writes nothing.
    pub(crate) fn write_bits(&mut self, value: u32, n: u32) {
        if n == 0 || n > MAX_BITS_PER_OP {
            return;
        }
        let mask = (1u64 << n) - 1;
        self.acc |= (u64::from(value) & mask) << self.bits_in_acc;
        self.bits_in_acc += n;
        while self.bits_in_acc >= 8 {
            self.buf.push((self.acc & 0xff) as u8);
            self.acc >>= 8;
            self.bits_in_acc -= 8;
        }
    }

    /// Pads the current partial byte with zero bits so the stream is byte-aligned. A no-op when
    /// already aligned. Used before a stored block's `LEN`/`NLEN`/data (RFC 1951 §3.2.4).
    pub(crate) fn align_to_byte(&mut self) {
        if self.bits_in_acc > 0 {
            self.buf.push((self.acc & 0xff) as u8);
            self.acc = 0;
            self.bits_in_acc = 0;
        }
    }

    /// Appends raw bytes. Requires the writer to be byte-aligned — call [`align_to_byte`] first.
    ///
    /// [`align_to_byte`]: BitWriter::align_to_byte
    pub(crate) fn write_bytes(&mut self, bytes: &[u8]) {
        debug_assert_eq!(self.bits_in_acc, 0, "write_bytes requires byte alignment");
        self.buf.extend_from_slice(bytes);
    }

    /// Flushes any partial trailing byte (zero-padding its high bits) and returns the bytes.
    pub(crate) fn finish(mut self) -> Vec<u8> {
        self.align_to_byte();
        self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_bit_is_zero_padded() {
        let mut w = BitWriter::new();
        w.write_bits(0b1, 1);
        assert_eq!(w.finish(), vec![0b0000_0001]);
    }

    #[test]
    fn packs_lsb_first_across_a_byte() {
        // Write 0b101 (3 bits) then 0b11010 (5 bits) -> one byte 0b11010_101 = 0xD5.
        let mut w = BitWriter::new();
        w.write_bits(0b101, 3);
        w.write_bits(0b11010, 5);
        assert_eq!(w.finish(), vec![0b1101_0101]);
    }

    #[test]
    fn ignores_out_of_range_widths() {
        let mut w = BitWriter::new();
        w.write_bits(0xFF, 0);
        w.write_bits(0xFF, 33);
        assert_eq!(w.finish(), Vec::<u8>::new());
    }

    #[test]
    fn align_then_write_bytes() {
        // Mirrors the stored-block layout: 3 header bits, align, then raw bytes.
        let mut w = BitWriter::new();
        w.write_bits(0b1, 1); // BFINAL
        w.write_bits(0b00, 2); // BTYPE = stored
        w.align_to_byte();
        w.write_bytes(&[0xAA, 0xBB]);
        // Header byte has BFINAL in bit 0; BTYPE bits 1-2 are zero; rest zero-padded.
        assert_eq!(w.finish(), vec![0b0000_0001, 0xAA, 0xBB]);
    }

    #[test]
    fn align_is_noop_when_aligned() {
        let mut w = BitWriter::new();
        w.write_bits(0xCD, 8);
        w.align_to_byte();
        w.align_to_byte();
        w.write_bytes(&[0x01]);
        assert_eq!(w.finish(), vec![0xCD, 0x01]);
    }

    #[test]
    fn wide_values_round_trip_through_bytes() {
        let mut w = BitWriter::new();
        w.write_bits(0xDEAD_BEEF, 32);
        assert_eq!(w.finish(), 0xDEAD_BEEFu32.to_le_bytes());
    }
}
