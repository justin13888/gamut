//! Packing raw integer samples into the uncompressed DNG sample stream and back.
//!
//! DNG stores uncompressed samples by bit depth (TIFF 6.0 / DNG):
//! - **8-bit**: one byte per sample.
//! - **16-bit**: two bytes per sample, in the file's byte order.
//! - **other depths** (e.g. 10/12/14-bit): samples packed **MSB-first**, with each *row* padded to
//!   a byte boundary (rows begin on byte boundaries). This sub-byte layout is byte-order-independent
//!   and is shared with the other TIFF-family formats, so it lives in [`gamut_bitstream`]; only the
//!   8/16-bit whole-byte paths below depend on the file's byte order.
//!
//! `samples_per_row` is `width * samples_per_pixel` (the per-plane samples interleave for
//! `LinearRaw`, so they pack/unpack identically to a single wider row of values).

use gamut_bitstream::{pack_msb_rows, unpack_msb_rows};
use gamut_ifd::ByteOrder;

/// Packs `samples` to the uncompressed DNG byte stream at `bits` per sample.
#[must_use]
pub(crate) fn pack(
    samples: &[u16],
    bits: u16,
    samples_per_row: usize,
    order: ByteOrder,
) -> Vec<u8> {
    match bits {
        8 => samples.iter().map(|&s| s as u8).collect(),
        16 => {
            let mut out = Vec::with_capacity(samples.len() * 2);
            for &s in samples {
                out.extend_from_slice(&order.pack_u16(s));
            }
            out
        }
        _ => pack_msb_rows(samples, bits, samples_per_row),
    }
}

/// Unpacks `count` samples (`rows` rows of `samples_per_row`) from the DNG byte stream at `bits`.
///
/// Returns up to `rows * samples_per_row` samples (fewer if `bytes` is short).
#[must_use]
pub(crate) fn unpack(
    bytes: &[u8],
    bits: u16,
    samples_per_row: usize,
    rows: usize,
    order: ByteOrder,
) -> Vec<u16> {
    match bits {
        8 => bytes.iter().map(|&b| u16::from(b)).collect(),
        16 => bytes
            .chunks_exact(2)
            .map(|c| order.u16([c[0], c[1]]))
            .collect(),
        _ => unpack_msb_rows(bytes, bits, samples_per_row, rows),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gamut_bitstream::row_bytes;

    fn roundtrip(bits: u16, spr: usize, rows: usize, order: ByteOrder) {
        let max = ((1u32 << bits) - 1) as u16;
        let samples: Vec<u16> = (0..spr * rows)
            .map(|i| (i as u16).wrapping_mul(2749) & max)
            .collect();
        let packed = pack(&samples, bits, spr, order);
        // MSB-packed rows are byte-aligned; 8/16-bit are exact multiples.
        if !matches!(bits, 8 | 16) {
            assert_eq!(packed.len(), row_bytes(spr, bits) * rows);
        }
        let got = unpack(&packed, bits, spr, rows, order);
        assert_eq!(got, samples, "bits={bits} spr={spr} rows={rows}");
    }

    #[test]
    fn roundtrips_all_depths() {
        for &order in &[ByteOrder::LittleEndian, ByteOrder::BigEndian] {
            for &bits in &[8u16, 10, 12, 14, 16] {
                roundtrip(bits, 7, 5, order); // odd width exercises row padding
                roundtrip(bits, 16, 3, order);
            }
        }
    }

    #[test]
    fn sixteen_bit_honours_byte_order() {
        // 0x1234 -> LE bytes 34 12, BE bytes 12 34; round-trips back under the same order.
        let le = pack(&[0x1234], 16, 1, ByteOrder::LittleEndian);
        let be = pack(&[0x1234], 16, 1, ByteOrder::BigEndian);
        assert_eq!(le, vec![0x34, 0x12]);
        assert_eq!(be, vec![0x12, 0x34]);
        assert_eq!(unpack(&le, 16, 1, 1, ByteOrder::LittleEndian), vec![0x1234]);
        assert_eq!(unpack(&be, 16, 1, 1, ByteOrder::BigEndian), vec![0x1234]);
    }

    #[test]
    fn eight_bit_is_one_byte_per_sample_regardless_of_row_geometry() {
        // The 8-bit fast path emits exactly one byte per sample and decodes one sample per byte,
        // independent of `samples_per_row`/`rows` — unlike the general MSB path, which pads each row
        // and drops a trailing partial row. A sample count that is *not* a multiple of the row width
        // pins that distinction (the general path would drop the last value).
        let samples = [0x00u16, 0x7F, 0xFF, 0x42, 0x10];
        let packed = pack(&samples, 8, 2, ByteOrder::LittleEndian);
        assert_eq!(packed, vec![0x00, 0x7F, 0xFF, 0x42, 0x10]);
        assert_eq!(
            unpack(&packed, 8, 2, 2, ByteOrder::LittleEndian),
            vec![0x00, 0x7F, 0xFF, 0x42, 0x10]
        );
    }
}
