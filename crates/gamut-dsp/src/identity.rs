//! AV1 1-D inverse and forward identity transforms for `TX` lengths 4/8/16/32 (AV1 §7.13.2.11–.15).
//!
//! The "identity" transform does not mix samples — it is a pure per-element scaling whose factor
//! grows by `√2` with each size doubling (`√2, 2, 2√2, 4` for 4/8/16/32). It backs the `IDTX`,
//! `V_*`, and `H_*` transform types used for screen content.
//!
//! [`inverse_identity`] transcribes the normative scaling (the decoder transform). [`forward_identity`]
//! applies the reciprocal scale so the pair reconstructs; because the scale is not a power of two for
//! every size, the forward rounds (an inherent, small loss reconciled with the 2-D shifts in AV1
//! §7.13.3).

use crate::butterfly::round2;

/// In-place 1-D inverse identity transform of the length-`2^n` array `t` (`2 ≤ n ≤ 5`), per
/// AV1 §7.13.2.15.
///
/// # Panics
/// Panics if `t.len() < (1 << n)` or `n` is outside `2..=5`.
pub fn inverse_identity(t: &mut [i64], n: u32) {
    let len = 1usize << n;
    assert!(t.len() >= len, "inverse_identity: array shorter than 2^n");
    match n {
        2 => {
            for v in &mut t[..len] {
                *v = round2(*v * 5793, 12); // × √2
            }
        }
        3 => {
            for v in &mut t[..len] {
                *v *= 2;
            }
        }
        4 => {
            for v in &mut t[..len] {
                *v = round2(*v * 11586, 12); // × 2√2
            }
        }
        5 => {
            for v in &mut t[..len] {
                *v *= 4;
            }
        }
        _ => panic!("inverse_identity: n must be in 2..=5"),
    }
}

/// In-place 1-D forward identity transform of the length-`2^n` array `t` (`2 ≤ n ≤ 5`): the
/// reciprocal per-element scaling that pairs with [`inverse_identity`].
///
/// # Panics
/// Panics if `t.len() < (1 << n)` or `n` is outside `2..=5`.
pub fn forward_identity(t: &mut [i64], n: u32) {
    let len = 1usize << n;
    assert!(t.len() >= len, "forward_identity: array shorter than 2^n");
    match n {
        2 => {
            for v in &mut t[..len] {
                *v = round2(*v * 2896, 12); // × 1/√2 (4096/5793 ≈ 2896/4096)
            }
        }
        3 => {
            for v in &mut t[..len] {
                *v = round2(*v, 1); // ÷ 2
            }
        }
        4 => {
            for v in &mut t[..len] {
                *v = round2(*v * 1448, 12); // × 1/(2√2)
            }
        }
        5 => {
            for v in &mut t[..len] {
                *v = round2(*v, 2); // ÷ 4
            }
        }
        _ => panic!("forward_identity: n must be in 2..=5"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            self.0
        }
        fn coeff(&mut self, range: i64) -> i64 {
            (self.next() >> 33) as i64 % (2 * range + 1) - range
        }
    }

    #[test]
    fn inverse_scales_match_spec_constants() {
        let mut t = [4i64, -8, 12, 16, 0, 0, 0, 0];
        inverse_identity(&mut t[..4], 2);
        assert_eq!(t[0], round2(4 * 5793, 12));
        let mut t8 = [3i64; 8];
        inverse_identity(&mut t8, 3);
        assert!(t8.iter().all(|&v| v == 6));
        let mut t32 = [5i64; 32];
        inverse_identity(&mut t32, 5);
        assert!(t32.iter().all(|&v| v == 20));
    }

    #[test]
    fn forward_then_inverse_recovers_within_rounding() {
        // The non-power-of-two scales (and the ÷2 / ÷4) round, so the round trip is exact only to
        // within a small tolerance — the inherent identity-transform loss.
        let mut rng = Lcg(0xfadd_1357_2468_9bdf);
        for n in 2..=5u32 {
            let len = 1usize << n;
            for _ in 0..500 {
                let resid: Vec<i64> = (0..len).map(|_| rng.coeff(255)).collect();
                let mut t = resid.clone();
                forward_identity(&mut t[..len], n);
                inverse_identity(&mut t[..len], n);
                for (m, (&got, &want)) in t[..len].iter().zip(&resid).enumerate() {
                    assert!(
                        (got - want).abs() <= 2,
                        "n={n} entry {m}: round trip {got} vs {want}",
                    );
                }
            }
        }
    }
}
