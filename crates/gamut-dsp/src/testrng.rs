//! The deterministic generator the transform tests drive their sweeps with.
//!
//! Five modules — `av1::{adst, dct, identity, wht}` and `jpeg::dct` — each carried a byte-identical
//! copy of this LCG in their own `#[cfg(test)] mod tests`. One copy means the sequence is defined
//! once: a transform sweep that "passes" because its generator degenerated is a failure mode no
//! individual test can see, and five copies are five chances for one of them to drift into it.
//!
//! Deliberately not a dependency on `rand`: every sweep here pins an explicit seed and must
//! reproduce byte-for-byte across runs and across the mutation survey, which a crate whose
//! generator may change between versions cannot promise. It is also `#[cfg(test)]`-only, so it
//! adds nothing to the shipped crate.

/// A 64-bit linear congruential generator with the Knuth/PCG multiplier.
///
/// Seeded explicitly by every caller; the constants are the ones all five copies used, so every
/// existing sweep sees exactly the sequence it saw before.
pub(crate) struct Lcg(u64);

impl Lcg {
    /// Starts the sequence at `seed`.
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The next raw 64-bit state.
    ///
    /// The low bits of an LCG are notoriously poor, so every derived helper below takes from the
    /// high end.
    pub(crate) fn next(&mut self) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        self.0
    }

    /// A transform coefficient in `[-range, range]`.
    pub(crate) fn coeff(&mut self, range: i64) -> i64 {
        (self.next() >> 33) as i64 % (2 * range + 1) - range
    }

    /// A level-shifted sample for a `precision`-bit source: `[-2^(P-1), 2^(P-1) - 1]`.
    ///
    /// The domain JPEG §A.3.1 defines for the forward DCT's input, covering both `P = 8` and the
    /// 12-bit extended sequential case.
    pub(crate) fn level_shifted_sample(&mut self, precision: u32) -> i32 {
        let span = 1_i64 << precision;
        let half = 1_i64 << (precision - 1);
        (((self.next() >> 40) as i64 % span) - half) as i32
    }
}

#[cfg(test)]
mod tests {
    use super::Lcg;

    #[test]
    fn the_sequence_is_reproducible_from_its_seed() {
        // The whole point of a hand-rolled generator over `rand`: the same seed gives the same
        // sweep on every run and inside every mutant scenario.
        let first: Vec<u64> = (0..8)
            .scan(Lcg::new(0x1234_5678_9abc_def0), |rng, _| Some(rng.next()))
            .collect();
        let second: Vec<u64> = (0..8)
            .scan(Lcg::new(0x1234_5678_9abc_def0), |rng, _| Some(rng.next()))
            .collect();

        assert_eq!(first, second);
    }

    #[test]
    fn different_seeds_give_different_sequences() {
        // A generator that ignored its seed would make every module's sweep run the same inputs
        // while still passing.
        let a: Vec<u64> = (0..8)
            .scan(Lcg::new(1), |rng, _| Some(rng.next()))
            .collect();
        let b: Vec<u64> = (0..8)
            .scan(Lcg::new(2), |rng, _| Some(rng.next()))
            .collect();

        assert_ne!(a, b);
    }

    #[test]
    fn coeff_stays_inside_the_requested_range() {
        let mut rng = Lcg::new(0xfadd_1357_2468_9bdf);

        for _ in 0..4096 {
            let value = rng.coeff(255);
            assert!((-255..=255).contains(&value), "{value} escaped [-255, 255]");
        }
    }

    #[test]
    fn coeff_reaches_both_signs_and_the_extremes() {
        // A generator collapsed to zero, or to one sign, would leave every transform sweep
        // exercising a single trivial input while still passing.
        let mut rng = Lcg::new(0x51a7_c0de_1234_5678);
        let values: Vec<i64> = (0..4096).map(|_| rng.coeff(255)).collect();

        assert!(
            values.iter().any(|&v| v < -200),
            "never went strongly negative"
        );
        assert!(
            values.iter().any(|&v| v > 200),
            "never went strongly positive"
        );
    }

    #[test]
    fn level_shifted_samples_span_the_precision_they_are_asked_for() {
        // §A.3.1: an 8-bit source is level-shifted to [-128, 127], a 12-bit one to [-2048, 2047].
        let mut rng = Lcg::new(0x1234_5678_9abc_def0);
        for _ in 0..4096 {
            let eight = rng.level_shifted_sample(8);
            assert!((-128..=127).contains(&eight), "{eight} escaped 8-bit range");
            let twelve = rng.level_shifted_sample(12);
            assert!(
                (-2048..=2047).contains(&twelve),
                "{twelve} escaped 12-bit range"
            );
        }
    }
}
