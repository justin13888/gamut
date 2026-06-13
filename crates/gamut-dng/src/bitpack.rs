//! Packing raw integer samples into the uncompressed DNG sample stream and back.
//!
//! DNG stores uncompressed samples by bit depth (TIFF 6.0 / DNG):
//! - **8-bit**: one byte per sample.
//! - **16-bit**: two bytes per sample, in the file's byte order.
//! - **other depths** (e.g. 10/12/14-bit): samples packed **MSB-first**, with each *row* padded to
//!   a byte boundary (rows begin on byte boundaries). The bit packing is byte-order-independent.
//!
//! `samples_per_row` is `width * samples_per_pixel` (the per-plane samples interleave for
//! `LinearRaw`, so they pack/unpack identically to a single wider row of values).

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

/// The byte length of one MSB-packed row of `samples_per_row` samples at `bits` each.
fn row_bytes(samples_per_row: usize, bits: u16) -> usize {
    (samples_per_row * usize::from(bits)).div_ceil(8)
}

/// MSB-first bit packing with byte-aligned rows.
fn pack_msb_rows(samples: &[u16], bits: u16, samples_per_row: usize) -> Vec<u8> {
    if samples_per_row == 0 {
        return Vec::new();
    }
    let rows = samples.len() / samples_per_row;
    let rb = row_bytes(samples_per_row, bits);
    let mut out = vec![0u8; rb * rows];
    for r in 0..rows {
        let base = r * rb;
        let mut bit = 0usize;
        for c in 0..samples_per_row {
            let v = samples[r * samples_per_row + c];
            for k in (0..bits).rev() {
                if (v >> k) & 1 == 1 {
                    out[base + bit / 8] |= 0x80 >> (bit % 8);
                }
                bit += 1;
            }
        }
    }
    out
}

/// Inverse of [`pack_msb_rows`].
fn unpack_msb_rows(bytes: &[u8], bits: u16, samples_per_row: usize, rows: usize) -> Vec<u16> {
    if samples_per_row == 0 {
        return Vec::new();
    }
    let rb = row_bytes(samples_per_row, bits);
    let mut out = Vec::with_capacity(samples_per_row * rows);
    for r in 0..rows {
        let base = r * rb;
        if base + rb > bytes.len() {
            break;
        }
        let mut bit = 0usize;
        for _ in 0..samples_per_row {
            let mut v = 0u16;
            for _ in 0..bits {
                let b = (bytes[base + bit / 8] >> (7 - bit % 8)) & 1;
                v = (v << 1) | u16::from(b);
                bit += 1;
            }
            out.push(v);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn twelve_bit_packs_two_samples_into_three_bytes() {
        // 0xABC, 0xDEF -> MSB-first: AB CD EF.
        let packed = pack(&[0x0ABC, 0x0DEF], 12, 2, ByteOrder::LittleEndian);
        assert_eq!(packed, vec![0xAB, 0xCD, 0xEF]);
    }
}
