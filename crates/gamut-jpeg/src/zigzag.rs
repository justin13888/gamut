//! The T.81 §A.3.6 zig-zag coefficient ordering (Figure A.6).
//!
//! The 2-D DCT produces an 8×8 block of coefficients in *natural* (raster, row-major) order —
//! element `v·8 + u` is coefficient `S_vu`. Entropy coding and the DQT segment (§B.2.4.1) instead
//! serialize the 64 coefficients along the diagonal **zig-zag** sequence of §A.3.6, which orders
//! them roughly by increasing spatial frequency so that the high-frequency tail is a run of zeros
//! the run-length AC coder collapses efficiently.
//!
//! [`ZIGZAG`] maps a zig-zag position `k` (0..64) to the natural-order index it reads, i.e.
//! `natural[ZIGZAG[k]]` is the `k`-th coefficient in transmission order. Position 0 is always the
//! DC coefficient (natural index 0).

/// Zig-zag scan order (T.81 §A.3.6, Figure A.6): `ZIGZAG[k]` is the natural (row-major) index of
/// the `k`-th coefficient in zig-zag transmission order.
pub const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, //
    17, 24, 32, 25, 18, 11, 4, 5, //
    12, 19, 26, 33, 40, 48, 41, 34, //
    27, 20, 13, 6, 7, 14, 21, 28, //
    35, 42, 49, 56, 57, 50, 43, 36, //
    29, 22, 15, 23, 30, 37, 44, 51, //
    58, 59, 52, 45, 38, 31, 39, 46, //
    53, 60, 61, 54, 47, 55, 62, 63, //
];

#[cfg(test)]
mod tests {
    use super::ZIGZAG;

    #[test]
    fn is_a_permutation_of_0_63() {
        // Every natural index 0..64 appears exactly once — a bijection, so no coefficient is dropped
        // or duplicated when reordering. A single mutated entry breaks the "each seen once" check.
        let mut seen = [false; 64];
        for &idx in &ZIGZAG {
            assert!(!seen[idx], "index {idx} appears twice");
            seen[idx] = true;
        }
        assert!(seen.iter().all(|&s| s), "not all indices covered");
    }

    #[test]
    fn dc_first_and_corner_last() {
        // The DC term leads the scan; the highest-frequency corner (natural 63) ends it.
        assert_eq!(ZIGZAG[0], 0);
        assert_eq!(ZIGZAG[63], 63);
    }

    #[test]
    fn early_diagonal_matches_spec() {
        // The first anti-diagonal traversal of Figure A.6: (0,0) → (0,1) → (1,0) → (2,0) → (1,1) →
        // (0,2) …, i.e. natural indices 0,1,8,16,9,2. Pins the row/column stride (8) and direction.
        assert_eq!(&ZIGZAG[..6], &[0, 1, 8, 16, 9, 2]);
    }
}
