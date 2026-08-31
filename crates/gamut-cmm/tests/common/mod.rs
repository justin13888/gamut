//! The deterministic generator the six Little-CMS differential suites drive their sweeps with.
//!
//! Each of `oracle_{clut,conformance,curves,intents,lut,shaper}.rs` carried its own copy of the
//! same LCG. The cores were identical — same multiplier, same increment, same `>> 33` — and only
//! the derived helpers differed.
//!
//! One copy matters more here than tidiness. `next_unit` meant **two different things**: in
//! `oracle_clut.rs` it returned roughly `[-0.05, 1.05]`, deliberately overshooting so the sweep
//! crosses the clamp edges, and in the other four it returned `[0, 1]`. A reader moving between
//! two of these files would have had every reason to assume otherwise, and a sweep that quietly
//! stopped covering its clamp edges is exactly the kind of thing no individual assertion notices.
//! The two meanings now have two names.

#![allow(dead_code)] // each integration-test binary uses a different subset

/// A 64-bit linear congruential generator with the Knuth/PCG multiplier.
///
/// Deliberately not a dependency on `rand`: every sweep pins an explicit seed and must reproduce
/// byte-for-byte across runs and across the mutation survey, which a crate whose generator may
/// change between versions cannot promise. The constants are the ones all six copies used, so
/// every existing sweep sees exactly the sequence it saw before.
pub struct Lcg(u64);

impl Lcg {
    /// Starts the sequence at `seed`.
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// The next 32 bits, taken from the high end — an LCG's low bits are poor.
    pub fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }

    /// The next 16 bits — a PCS-encoded sample.
    pub fn next_u16(&mut self) -> u16 {
        (self.next_u32() & 0xFFFF) as u16
    }

    /// A sample in `[0, 0.5)`.
    ///
    /// **Not `[0, 1]`, despite the name and the six copies this replaces.** [`Self::next_u32`]
    /// takes its output from `self.0 >> 33`, which leaves 31 bits, so the largest value it can
    /// return is `2^31 - 1` -- and this divides by `u32::MAX`, which is `2^32 - 1`. The quotient
    /// therefore never exceeds 0.4999999999.
    ///
    /// The behaviour is preserved exactly as the six copies had it, so every existing sweep sees
    /// the sequence it has always seen. It is **not** correct, and #453 tracks it: all six
    /// Little-CMS differential suites have only ever swept the lower half of their input domain,
    /// and correcting the divisor pushes `conformance_pairs_battery` past `LOOSE_LUT_BOUND`.
    /// Widening the domain and re-calibrating that bound is a judgement about acceptable colour
    /// error against the reference CMM, which is why it is filed rather than done here.
    pub fn next_unit(&mut self) -> f64 {
        f64::from(self.next_u32()) / f64::from(u32::MAX)
    }

    /// A sample in `[-0.05, 0.5)`, exercising the **lower** clamp edge alongside the interior.
    ///
    /// Named apart from [`Self::next_unit`] on purpose: the overshoot is the point of it, and
    /// `oracle_clut.rs` is the suite whose subject is what happens outside the unit interval.
    ///
    /// The copy this replaces documented it as `[-0.05, 1.05]` and `exercising the clamp edges`,
    /// plural. It never reached the upper one: `next_unit` caps at 0.5 (see there), so
    /// `0.5 * 1.1 - 0.05` is also 0.5. The upper clamp has never been tested. Part of #453.
    pub fn next_unit_with_overshoot(&mut self) -> f64 {
        self.next_unit() * 1.1 - 0.05
    }
}

#[cfg(test)]
mod tests {
    use super::Lcg;

    #[test]
    fn the_sequence_is_reproducible_from_its_seed() {
        let first: Vec<u32> = (0..8)
            .scan(Lcg::new(0x1234_5678_9abc_def0), |rng, _| {
                Some(rng.next_u32())
            })
            .collect();
        let second: Vec<u32> = (0..8)
            .scan(Lcg::new(0x1234_5678_9abc_def0), |rng, _| {
                Some(rng.next_u32())
            })
            .collect();

        assert_eq!(first, second);
    }

    #[test]
    fn different_seeds_give_different_sequences() {
        let a: Vec<u32> = (0..8)
            .scan(Lcg::new(1), |r, _| Some(r.next_u32()))
            .collect();
        let b: Vec<u32> = (0..8)
            .scan(Lcg::new(2), |r, _| Some(r.next_u32()))
            .collect();

        assert_ne!(a, b);
    }

    #[test]
    fn next_unit_covers_only_the_lower_half_of_the_unit_interval() {
        // Pins the defect #453 records, so it cannot drift further while it is open and so the
        // number in the doc comment above is checkable rather than asserted in prose. The upper
        // bound is 0.5 because `next_u32` yields 31 bits and this divides by a 32-bit maximum.
        let mut rng = Lcg::new(0x51a7_c0de_1234_5678);
        let values: Vec<f64> = (0..8192).map(|_| rng.next_unit()).collect();

        assert!(
            values.iter().all(|&v| (0.0..0.5).contains(&v)),
            "outside [0, 0.5)"
        );
        // And it does reach most of that half, so the sweeps are not degenerate as well as narrow.
        assert!(values.iter().any(|&v| v > 0.49), "never approached 0.5");
        assert!(values.iter().any(|&v| v < 0.01), "never approached 0");
    }

    #[test]
    fn next_unit_with_overshoot_reaches_below_zero_but_not_above_one() {
        // Half of what its name and its original comment claimed. The lower clamp edge is
        // exercised; the upper one never is. Part of #453.
        let mut rng = Lcg::new(0x0bad_f00d_dead_0001);
        let values: Vec<f64> = (0..8192).map(|_| rng.next_unit_with_overshoot()).collect();

        assert!(values.iter().any(|&v| v < 0.0), "never went below 0");
        assert!(
            values.iter().all(|&v| v <= 0.5),
            "the upper clamp is unexpectedly reachable -- has #453 been fixed?"
        );
    }
}
