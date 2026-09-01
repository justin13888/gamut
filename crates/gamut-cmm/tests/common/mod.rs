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
//!
//! Neither of them did what it said. All six copies divided a **31-bit** `next_u32` by
//! `u32::MAX`, so `next_unit` capped at 0.4999999999 and every sweep driven by it had only ever
//! covered the lower half of its domain -- and `oracle_clut.rs`'s overshoot variant, documented as
//! reaching `1.05`, never crossed 1.0 at all. That is #453, and it is fixed here: the divisor is
//! now the generator's own maximum, so `next_unit` spans `[0, 1]` and the overshoot variant spans
//! `[-0.05, 1.05]` as both always claimed. Every bound the widened sweeps run against was
//! re-measured in the same change.

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

    /// The largest value [`Self::next_u32`] can return.
    ///
    /// `next_u32` takes its output from `self.0 >> 33`, which leaves **31** bits, so its maximum
    /// is `2^31 - 1` and not `u32::MAX`. Dividing by the wrong one of those is the whole of #453:
    /// every sweep in this crate had only ever covered the lower half of its input domain.
    const NEXT_U32_MAX: u32 = (1 << 31) - 1;

    /// A sample in `[0, 1]`.
    ///
    /// Divides by [`Self::NEXT_U32_MAX`], the generator's actual maximum. The six copies this
    /// replaces divided by `u32::MAX` and so capped at 0.4999999999 (#453); the bounds every
    /// suite asserts against were re-measured over the widened domain in the same change.
    pub fn next_unit(&mut self) -> f64 {
        f64::from(self.next_u32()) / f64::from(Self::NEXT_U32_MAX)
    }

    /// A sample in `[-0.05, 1.05]`, exercising **both** clamp edges alongside the interior.
    ///
    /// Named apart from [`Self::next_unit`] on purpose: the overshoot is the point of it, and
    /// `oracle_clut.rs` is the suite whose subject is what happens outside the unit interval.
    ///
    /// With the pre-#453 divisor this reached `-0.05` but never went above `0.5`, so the upper
    /// clamp -- the edge the copy's own comment claimed it was testing -- had never been exercised
    /// at all.
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
    fn next_unit_covers_the_whole_unit_interval() {
        // The regression guard for #453. Before it, this generator capped at 0.4999999999 and no
        // suite noticed, because every assertion was a one-sided bound on an error the narrowed
        // corpus never provoked. Both halves are asserted: inside [0, 1], and actually reaching
        // each end of it -- a divisor that is merely too large keeps the range legal while making
        // the sweep degenerate.
        let mut rng = Lcg::new(0x51a7_c0de_1234_5678);
        let values: Vec<f64> = (0..8192).map(|_| rng.next_unit()).collect();

        assert!(
            values.iter().all(|&v| (0.0..=1.0).contains(&v)),
            "outside [0, 1]"
        );
        assert!(values.iter().any(|&v| v > 0.99), "never approached 1");
        assert!(values.iter().any(|&v| v < 0.01), "never approached 0");
        // The upper half is the half that was missing, so it is pinned on its own.
        assert!(
            values.iter().filter(|&&v| v > 0.5).count() > 3_000,
            "the upper half of the domain is underpopulated -- has #453 regressed?"
        );
    }

    #[test]
    fn next_unit_with_overshoot_reaches_both_clamp_edges() {
        // The other half of #453: this had never crossed 1.0, despite its own comment saying the
        // clamp edges, plural, were the point of it. `oracle_clut.rs` is the suite that cares.
        let mut rng = Lcg::new(0x0bad_f00d_dead_0001);
        let values: Vec<f64> = (0..8192).map(|_| rng.next_unit_with_overshoot()).collect();

        assert!(values.iter().any(|&v| v < 0.0), "never went below 0");
        assert!(values.iter().any(|&v| v > 1.0), "never went above 1");
        assert!(
            values.iter().all(|&v| (-0.05..=1.05).contains(&v)),
            "outside [-0.05, 1.05]"
        );
    }
}
