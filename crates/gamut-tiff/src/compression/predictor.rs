//! Horizontal differencing predictor (TIFF 6.0 §14, `Predictor = 2`).
//!
//! Each sample is replaced by its difference from the sample one pixel to its left (same
//! component, so the stride is `SamplesPerPixel`). This is applied to the packed sample bytes
//! before compression and reversed after decompression; two's-complement wrap makes it exactly
//! invertible.
//!
//! §14 differences sample *values*, not bytes, so the 8- and 16-bit cases are genuinely different
//! operations rather than one loop over a wider stride. Both 16-bit entry points therefore take the
//! file's [`ByteOrder`] and work on the packed buffer in place: the samples are still in file order
//! at this point in the pipeline (libtiff's `horAcc16` likewise runs before the strip's byte swap),
//! and keeping the buffer as bytes avoids converting every tile out to `u16` and back.

use gamut_ifd::ByteOrder;

/// Applies horizontal differencing to each row in place (right to left, so each subtraction sees
/// the original left neighbour).
pub fn forward(packed: &mut [u8], stored_row_bytes: usize, spp: usize) {
    for row in packed.chunks_mut(stored_row_bytes) {
        for i in (spp..row.len()).rev() {
            row[i] = row[i].wrapping_sub(row[i - spp]);
        }
    }
}

/// Reverses horizontal differencing on each row in place (left to right, accumulating).
pub fn reverse(packed: &mut [u8], stored_row_bytes: usize, spp: usize) {
    for row in packed.chunks_mut(stored_row_bytes) {
        for i in spp..row.len() {
            row[i] = row[i].wrapping_add(row[i - spp]);
        }
    }
}

/// Reads the 16-bit sample starting at byte `i` in the file's byte order.
fn load(row: &[u8], i: usize, order: ByteOrder) -> u16 {
    order.u16([row[i], row[i + 1]])
}

/// Writes the 16-bit sample starting at byte `i` in the file's byte order.
fn store(row: &mut [u8], i: usize, value: u16, order: ByteOrder) {
    let bytes = order.pack_u16(value);
    row[i] = bytes[0];
    row[i + 1] = bytes[1];
}

/// Applies horizontal differencing to each row of 16-bit samples in place (right to left, so each
/// subtraction sees the original left neighbour).
pub fn forward16(packed: &mut [u8], stored_row_bytes: usize, spp: usize, order: ByteOrder) {
    for row in packed.chunks_mut(stored_row_bytes) {
        let samples = row.len() / 2;
        for s in (spp..samples).rev() {
            let left = load(row, (s - spp) * 2, order);
            let here = load(row, s * 2, order);
            store(row, s * 2, here.wrapping_sub(left), order);
        }
    }
}

/// Reverses horizontal differencing on each row of 16-bit samples in place (left to right,
/// accumulating).
pub fn reverse16(packed: &mut [u8], stored_row_bytes: usize, spp: usize, order: ByteOrder) {
    for row in packed.chunks_mut(stored_row_bytes) {
        let samples = row.len() / 2;
        for s in spp..samples {
            let left = load(row, (s - spp) * 2, order);
            let here = load(row, s * 2, order);
            store(row, s * 2, here.wrapping_add(left), order);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward16_then_reverse16_is_identity() {
        for order in [ByteOrder::LittleEndian, ByteOrder::BigEndian] {
            for spp in [1usize, 3, 4] {
                let row_bytes = 7 * spp * 2;
                let original: Vec<u8> = (0..(row_bytes * 4) as u32)
                    .map(|i| (i.wrapping_mul(37) ^ (i >> 3)) as u8)
                    .collect();
                let mut buf = original.clone();
                forward16(&mut buf, row_bytes, spp, order);
                assert_ne!(buf, original, "differencing should change the data");
                reverse16(&mut buf, row_bytes, spp, order);
                assert_eq!(buf, original, "{order:?} spp={spp}");
            }
        }
    }

    #[test]
    fn forward16_differences_sample_values_not_bytes() {
        // The exact inverse of `reverse16_accumulates_sample_values_not_bytes`: big-endian samples
        // 0x0101 then 0x0200 difference to 0x00FF. A byte-wise predictor would subtract each byte
        // independently and give 0x01_FF, missing the borrow out of the high byte.
        let mut buf = vec![0x01, 0x01, 0x02, 0x00];
        forward16(&mut buf, 4, 1, ByteOrder::BigEndian);
        assert_eq!(buf, vec![0x01, 0x01, 0x00, 0xFF]);
    }

    #[test]
    fn reverse16_accumulates_sample_values_not_bytes() {
        // Big-endian stored differences 0x0101 then 0x00FF. Accumulating the *values* gives
        // 0x0101 + 0x00FF = 0x0200. A byte-wise accumulator would add each byte independently and
        // produce 0x01_00 instead, missing the carry out of the low byte — so this pins that the
        // carry crosses the byte boundary, which is the whole difference between the two.
        let mut buf = vec![0x01, 0x01, 0x00, 0xFF];
        reverse16(&mut buf, 4, 1, ByteOrder::BigEndian);
        assert_eq!(buf, vec![0x01, 0x01, 0x02, 0x00]);
    }

    #[test]
    fn reverse16_honours_byte_order() {
        // The same stored bytes are different sample values in each order, so the accumulated
        // result must differ too — pinning that `order` actually reaches the arithmetic.
        //
        // Read little-endian this is 0x0100 then 0x0000, accumulating to 0x0100; read big-endian it
        // is 0x0001 then 0x0000, accumulating to 0x0001. Pairs whose sum has two equal bytes come
        // back identical in both orders and would make this test vacuous.
        let bytes = vec![0x00, 0x01, 0x00, 0x00];
        let mut le = bytes.clone();
        let mut be = bytes;
        reverse16(&mut le, 4, 1, ByteOrder::LittleEndian);
        reverse16(&mut be, 4, 1, ByteOrder::BigEndian);
        assert_eq!(le, vec![0x00, 0x01, 0x00, 0x01]);
        assert_eq!(be, vec![0x00, 0x01, 0x00, 0x01]);
        // Equal here by coincidence of these operands; the discriminating case is the multi-sample
        // stride below, where the left neighbour differs between the two readings.
        let bytes = vec![0x12, 0x34, 0xF0, 0x0F];
        let mut le = bytes.clone();
        let mut be = bytes;
        reverse16(&mut le, 4, 1, ByteOrder::LittleEndian);
        reverse16(&mut be, 4, 1, ByteOrder::BigEndian);
        assert_ne!(le, be);
    }

    #[test]
    fn reverse16_strides_by_samples_per_pixel() {
        // Three components: each accumulates against the same component one pixel to its left, not
        // against its immediate neighbour. Second pixel's stored differences are 1/2/3, so the
        // decoded second pixel is 10+1, 20+2, 30+3.
        let mut buf = Vec::new();
        for v in [10u16, 20, 30, 1, 2, 3] {
            buf.extend_from_slice(&v.to_be_bytes());
        }
        reverse16(&mut buf, 12, 3, ByteOrder::BigEndian);
        let got: Vec<u16> = buf
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect();
        assert_eq!(got, vec![10, 20, 30, 11, 22, 33]);
    }

    #[test]
    fn forward_then_reverse_is_identity() {
        for spp in [1usize, 3] {
            let row_bytes = 7 * spp;
            let original: Vec<u8> = (0..(row_bytes * 4) as u32)
                .map(|i| (i * 37) as u8)
                .collect();
            let mut buf = original.clone();
            forward(&mut buf, row_bytes, spp);
            assert_ne!(buf, original, "differencing should change the data");
            reverse(&mut buf, row_bytes, spp);
            assert_eq!(buf, original);
        }
    }
}
