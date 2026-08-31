//! Unsigned LEB128 (little-endian base 128) integers (AV1 §4.10.5, Annex B `leb128()`).
//!
//! Used to carry `obu_size` in the low-overhead bitstream format. The spec permits up to 8 bytes;
//! this writer always emits the minimal-length encoding, which every conformant reader accepts.

/// Appends the minimal unsigned LEB128 encoding of `value` to `out`.
pub fn write_leb128(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

/// Returns the number of bytes the minimal unsigned LEB128 encoding of `value` occupies.
#[must_use]
pub fn leb128_len(mut value: u64) -> usize {
    let mut n = 1;
    while value >= 0x80 {
        value >>= 7;
        n += 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sweep both length and round-trip claims run over: each byte-count boundary
    /// (127/128), a multi-byte value, and the 32-bit ceiling.
    const VALUES: &[u64] = &[0, 1, 127, 128, 300, 0xffff, 0x10_0000, u32::MAX as u64];

    /// Minimal reference LEB128 reader for round-trip assertions.
    fn read_leb128(bytes: &[u8]) -> (u64, usize) {
        let mut value = 0u64;
        let mut i = 0;
        loop {
            let byte = bytes[i];
            value |= u64::from(byte & 0x7f) << (7 * i);
            i += 1;
            if byte & 0x80 == 0 {
                break;
            }
        }
        (value, i)
    }

    #[test]
    fn known_encodings() {
        let mut out = Vec::new();
        write_leb128(&mut out, 0);
        assert_eq!(out, &[0x00]);

        out.clear();
        write_leb128(&mut out, 127);
        assert_eq!(out, &[0x7f]);

        out.clear();
        write_leb128(&mut out, 128);
        assert_eq!(out, &[0x80, 0x01]);

        out.clear();
        write_leb128(&mut out, 0x3fff);
        assert_eq!(out, &[0xff, 0x7f]);
    }

    #[test]
    fn leb128_len_predicts_what_the_writer_emits() {
        // `leb128_len` computes the encoded size without encoding, so it is a second,
        // independent implementation of the same length rule -- not a round trip. A defect in
        // either one alone shows up here.
        for &v in VALUES {
            let mut out = Vec::new();
            write_leb128(&mut out, v);

            assert_eq!(out.len(), leb128_len(v), "len mismatch for {v}");
        }
    }

    #[test]
    fn reading_back_a_written_value_recovers_it_and_its_length() {
        for &v in VALUES {
            let mut out = Vec::new();
            write_leb128(&mut out, v);

            let (decoded, used) = read_leb128(&out);
            assert_eq!(decoded, v);
            assert_eq!(used, out.len(), "reader consumed the wrong span for {v}");
        }
    }
}
