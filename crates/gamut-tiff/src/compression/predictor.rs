//! Horizontal differencing predictor (TIFF 6.0 §14, `Predictor = 2`).
//!
//! Each sample is replaced by its difference from the sample one pixel to its left (same
//! component, so the stride is `SamplesPerPixel`). This is applied to the packed 8-bit sample
//! bytes before compression and reversed after decompression; two's-complement wrap makes it
//! exactly invertible. The predictor is defined for 8-bit samples only.

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

#[cfg(test)]
mod tests {
    use super::*;

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
