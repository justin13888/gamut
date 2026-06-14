//! Most-significant-bit-first packing of fixed-width integer samples into byte-aligned rows.
//!
//! TIFF-family raw formats (TIFF 6.0, DNG/TIFF-EP) store sub-byte sample depths (e.g. 10/12/14-bit)
//! by packing samples **MSB-first** and padding **each row to a byte boundary** (every row begins on
//! a fresh byte). The packing itself is byte-order-independent — endianness only governs the 8/16-bit
//! whole-byte paths, which callers handle themselves.
//!
//! `samples_per_row` is the number of samples in one packed row (for interleaved planes it is the
//! per-row sample count across all planes, which pack/unpack identically to one wider row).

/// The byte length of one MSB-packed row of `samples_per_row` samples at `bits` each.
#[must_use]
pub fn row_bytes(samples_per_row: usize, bits: u16) -> usize {
    (samples_per_row * usize::from(bits)).div_ceil(8)
}

/// Packs `samples` MSB-first at `bits` per sample, padding each row of `samples_per_row` to a byte
/// boundary. Returns `row_bytes(samples_per_row, bits) * rows`, where `rows = samples.len() /
/// samples_per_row` (a `samples_per_row` of `0` yields an empty buffer).
#[must_use]
pub fn pack_msb_rows(samples: &[u16], bits: u16, samples_per_row: usize) -> Vec<u8> {
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

/// Unpacks `rows` rows of `samples_per_row` samples at `bits` each from MSB-packed, row-padded
/// `bytes`. The inverse of [`pack_msb_rows`]; stops early (returning fewer samples) if `bytes` is
/// short, and yields nothing when `samples_per_row` is `0`.
#[must_use]
pub fn unpack_msb_rows(bytes: &[u8], bits: u16, samples_per_row: usize, rows: usize) -> Vec<u16> {
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
                // `+`, not `|`: `v << 1` always has a clear low bit, so `|`/`^`/`+` agree here — but
                // `+` is the form that stays mutation-testable (a `|`->`^` swap would be equivalent).
                v = (v << 1) + u16::from(b);
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

    fn roundtrip(bits: u16, spr: usize, rows: usize) {
        let max = ((1u32 << bits) - 1) as u16;
        let samples: Vec<u16> = (0..spr * rows)
            .map(|i| (i as u16).wrapping_mul(2749) & max)
            .collect();
        let packed = pack_msb_rows(&samples, bits, spr);
        assert_eq!(packed.len(), row_bytes(spr, bits) * rows);
        let got = unpack_msb_rows(&packed, bits, spr, rows);
        assert_eq!(got, samples, "bits={bits} spr={spr} rows={rows}");
    }

    #[test]
    fn roundtrips_sub_byte_depths() {
        for &bits in &[10u16, 12, 14] {
            roundtrip(bits, 7, 5); // odd width exercises row padding
            roundtrip(bits, 16, 3);
        }
    }

    #[test]
    fn twelve_bit_packs_two_samples_into_three_bytes() {
        // 0xABC, 0xDEF -> MSB-first: AB CD EF.
        let packed = pack_msb_rows(&[0x0ABC, 0x0DEF], 12, 2);
        assert_eq!(packed, vec![0xAB, 0xCD, 0xEF]);
    }

    #[test]
    fn rows_are_byte_aligned() {
        // 12-bit, 1 sample/row: each row is its own byte (4 bits padding) -> 2 bytes for 2 rows.
        let packed = pack_msb_rows(&[0xABC, 0xDEF], 12, 1);
        assert_eq!(packed.len(), 4);
        assert_eq!(packed, vec![0xAB, 0xC0, 0xDE, 0xF0]);
        assert_eq!(unpack_msb_rows(&packed, 12, 1, 2), vec![0xABC, 0xDEF]);
    }

    #[test]
    fn empty_row_width_yields_nothing() {
        assert!(pack_msb_rows(&[1, 2, 3], 12, 0).is_empty());
        assert!(unpack_msb_rows(&[0xAB, 0xCD], 12, 0, 4).is_empty());
    }

    #[test]
    fn unpack_stops_on_short_input() {
        // Two 12-bit rows of one sample need 4 bytes; give 2 -> only the first row decodes.
        assert_eq!(unpack_msb_rows(&[0xAB, 0xC0], 12, 1, 2), vec![0xABC]);
    }

    #[test]
    fn row_bytes_rounds_up() {
        assert_eq!(row_bytes(2, 12), 3); // 24 bits -> 3 bytes, exact
        assert_eq!(row_bytes(1, 12), 2); // 12 bits -> 2 bytes, padded
        assert_eq!(row_bytes(0, 12), 0);
    }
}
