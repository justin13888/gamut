//! Executable laws for the read ledger, shared by the property tests and the fuzz tier.
//!
//! Each function takes plain data, checks one law, and returns a [`Violation`] naming what broke
//! it. No test framework is involved and nothing here panics, so the same body runs under a
//! pinned-seed `proptest` in the per-PR gate and under a corpus-guided driver in extended CI
//! (issues #240, #264). That is the point: a property is the specification a fuzzer checks, and
//! writing it once is what keeps the two lanes from drifting apart.
//!
//! Every law takes ranges already inside [`UNIVERSE`], so an arbitrary byte string from a fuzz
//! driver maps to a bounded, cheap case rather than a multi-gigabyte allocation. Normalising is
//! the *caller's* job — [`normalise`] does it — deliberately: a law that normalised its own
//! inputs would feed the same value to the model and to the code under test, and a defect in the
//! normaliser would cancel on both sides. That is the self-consistency this whole policy warns
//! about, and the pilot should not model it.
//!
//! This module is the test oracle, not the system under test, so `.cargo/mutants.toml` excludes
//! `crates/*/src/invariants.rs` from mutation. Every new `invariants` module inherits that.

use crate::segment::Range;
use crate::track::ReadLedger;

/// The size of the offset space these laws are checked over.
///
/// Ledger arithmetic is uniform in the offset, so a bounded universe costs no generality while
/// keeping every case allocation-bounded for a fuzz driver.
pub const UNIVERSE: u64 = 4096;

/// The longest span a normalised input can describe.
const MAX_LEN: u64 = 256;

/// A law that did not hold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    law: &'static str,
    detail: String,
}

impl Violation {
    /// Builds a violation of `law`.
    fn new(law: &'static str, detail: String) -> Self {
        Self { law, detail }
    }

    /// The name of the law that was broken.
    #[must_use]
    pub fn law(&self) -> &'static str {
        self.law
    }

    /// What broke it, in terms the failing case can be read from.
    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

impl core::fmt::Display for Violation {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}: {}", self.law, self.detail)
    }
}

/// Maps an arbitrary range wholly inside [`UNIVERSE`], preserving zero length.
///
/// Callers hand the result to the laws below, which assume it. The result never runs past the
/// universe, so the model and the code under test see exactly the same bytes — clipping one but
/// not the other would manufacture disagreements that are artefacts of the harness.
///
/// Zero-length ranges survive normalisation deliberately: they are the input class that separates
/// a claim filter that drops them from one that lets them through.
#[must_use]
pub fn normalise(range: Range) -> Range {
    let start = range.start % UNIVERSE;
    Range {
        start,
        len: (range.len % (MAX_LEN + 1)).min(UNIVERSE - start),
    }
}

/// Marks every byte offset a normalised range covers.
fn mark(set: &mut [bool], range: Range) {
    let start = usize::try_from(range.start).unwrap_or(set.len());
    let end = usize::try_from(range.end()).unwrap_or(set.len());
    for byte in set.iter_mut().take(end).skip(start) {
        *byte = true;
    }
}

/// The byte set a range list covers.
fn to_set(ranges: &[Range]) -> Vec<bool> {
    let mut set = vec![false; UNIVERSE as usize];
    for &r in ranges {
        mark(&mut set, r);
    }
    set
}

/// Non-empty, strictly ascending, and never adjacent — the shape both the ledger and the
/// subtraction promise.
///
/// Adjacency is the interesting half: two touching ranges describe the same byte set as one, so
/// a coalescing defect is invisible to a set-equality check on its own.
fn canonical_form(ranges: &[Range], law: &'static str) -> Result<(), Violation> {
    for (i, r) in ranges.iter().enumerate() {
        if r.len == 0 {
            return Err(Violation::new(law, format!("empty range at index {i}")));
        }
        if let Some(prev) = i.checked_sub(1).and_then(|p| ranges.get(p))
            && prev.end() >= r.start
        {
            return Err(Violation::new(
                law,
                format!("{prev:?} and {r:?} are adjacent or overlapping"),
            ));
        }
    }
    Ok(())
}

/// Every span a ledger reports is non-empty, sorted and separated from its neighbour, the
/// recorded bytes are exactly the bytes read, and [`ReadLedger::contains`] agrees with them.
///
/// `reads` must already lie inside [`UNIVERSE`] — pass them through [`normalise`] first.
///
/// # Errors
///
/// Returns the first [`Violation`] found.
pub fn ledger_is_canonical(reads: &[Range]) -> Result<(), Violation> {
    let mut ledger = ReadLedger::new();
    let mut model = vec![false; UNIVERSE as usize];
    for &r in reads {
        ledger.record(r.start, r.len);
        mark(&mut model, r);
    }

    canonical_form(ledger.spans(), "ledger_is_canonical")?;

    if to_set(ledger.spans()) != model {
        return Err(Violation::new(
            "ledger_is_canonical",
            format!(
                "recorded bytes differ from the bytes read: spans {:?}",
                ledger.spans()
            ),
        ));
    }

    // `contains` must agree with the model at the boundaries of every recorded span — the
    // offsets a fencepost error lives at — rather than over some fixed window the generated
    // reads would usually miss. The zero-length query is included: it is vacuously contained.
    for span in ledger.spans() {
        for probe in [
            Range {
                start: span.start,
                len: 0,
            },
            Range {
                start: span.start,
                len: 1,
            },
            *span,
            Range {
                start: span.start,
                len: span.len + 1,
            },
            Range {
                start: span.start.saturating_sub(1),
                len: 2,
            },
            Range {
                start: span.end().saturating_sub(1),
                len: 2,
            },
        ] {
            let want = (probe.start..probe.end()).all(|b| model.get(b as usize) == Some(&true));
            if ledger.contains(probe) != want {
                return Err(Violation::new(
                    "ledger_is_canonical",
                    format!("contains({probe:?}) disagrees with the recorded bytes (want {want})"),
                ));
            }
        }
    }
    Ok(())
}

/// [`ReadLedger::subtract`] returns exactly the recorded bytes that no claim covers, in
/// canonical form — however the claims are ordered, overlapped, duplicated or split, and
/// including zero-length claims, which cover nothing and must therefore split nothing.
///
/// `reads` and `claims` must already lie inside [`UNIVERSE`] — pass them through [`normalise`]
/// first.
///
/// # Errors
///
/// Returns the first [`Violation`] found.
pub fn subtract_is_set_difference(reads: &[Range], claims: &[Range]) -> Result<(), Violation> {
    let mut ledger = ReadLedger::new();
    let mut read_bytes = vec![false; UNIVERSE as usize];
    for &r in reads {
        ledger.record(r.start, r.len);
        mark(&mut read_bytes, r);
    }

    let mut claimed = vec![false; UNIVERSE as usize];
    for &c in claims {
        mark(&mut claimed, c);
    }

    let out = ledger.subtract(claims);
    canonical_form(&out, "subtract_is_set_difference")?;

    let got = to_set(&out);
    for byte in 0..UNIVERSE as usize {
        let want = read_bytes[byte] && !claimed[byte];
        if got[byte] != want {
            return Err(Violation::new(
                "subtract_is_set_difference",
                format!(
                    "byte {byte}: subtract says {}, the set difference says {want} \
                     (spans {:?}, claims {claims:?})",
                    got[byte],
                    ledger.spans()
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use proptest::collection;
    use proptest::prelude::{Just, Strategy, prop_assert_eq, prop_oneof, proptest};
    use proptest::test_runner::{Config, RngSeed};

    use super::{
        MAX_LEN, Range, UNIVERSE, canonical_form, ledger_is_canonical, normalise,
        subtract_is_set_difference,
    };

    /// The workspace property-test configuration (`docs/testing.md`).
    ///
    /// The seed is **pinned**: the `--in-diff` mutation gate is blocking, so a test whose
    /// pass/fail depends on OS entropy would make cargo-mutants report CAUGHT or MISSED for the
    /// same mutant on different runs — and the equivalence proofs in `.cargo/mutants.toml` are
    /// only meaningful against a deterministic suite. The shrink bounds are finite because
    /// proptest's defaults are `u32::MAX` iterations and *no* wall-clock cap, and shrinking runs
    /// on the failing path, which is most of what a mutation survey does.
    ///
    /// Failure persistence is **off**, and that is part of the same contract. cargo-mutants
    /// reuses one tree copy across many mutants, restoring only the file it mutated — so a
    /// `proptest-regressions` file written by one mutant's failure would survive into the next
    /// mutant's run and be replayed first, making CAUGHT/MISSED depend on mutant ordering and
    /// shard assignment. A shrunk counterexample is promoted into a named deterministic test
    /// instead, which is where its regression value belongs.
    fn config() -> Config {
        Config {
            cases: 512,
            max_shrink_iters: 2048,
            max_shrink_time: 10_000,
            rng_seed: RngSeed::Fixed(0x6761_6D75_745F_6966),
            failure_persistence: None,
            ..Config::default()
        }
    }

    /// A range inside [`UNIVERSE`], ready to hand to a law.
    ///
    /// Normalising here rather than inside the laws is deliberate: a law that normalised its own
    /// inputs would feed the same value to the model and to the code under test, so a defect in
    /// the normaliser would cancel on both sides.
    ///
    /// Zero length is drawn deliberately often (~1 in 7): a zero-length *claim* covers nothing
    /// and must therefore split nothing, and that is the input class separating a claim filter
    /// that drops empty ranges from one that keeps them. Uniform `u64` lengths would reach it
    /// about once in 257 draws.
    fn range() -> impl Strategy<Value = Range> {
        (
            0..UNIVERSE,
            prop_oneof![1 => Just(0u64), 6 => 1u64..=MAX_LEN],
        )
            .prop_map(|(start, len)| normalise(Range { start, len }))
    }

    proptest! {
        #![proptest_config(config())]

        /// `record` coalesces into a canonical span set that holds exactly the bytes read.
        #[test]
        fn recorded_spans_are_canonical(reads in collection::vec(range(), 0..24)) {
            prop_assert_eq!(ledger_is_canonical(&reads), Ok(()));
        }

        /// `subtract` is the set difference, in canonical form, for any claim list.
        #[test]
        fn subtract_is_the_set_difference(
            reads in collection::vec(range(), 0..24),
            claims in collection::vec(range(), 0..24),
        ) {
            prop_assert_eq!(subtract_is_set_difference(&reads, &claims), Ok(()));
        }

    }

    // The laws' own failure paths. A law that cannot report a violation is not a law, and under
    // an unmutated `track.rs` the properties above only ever exercise the `Ok` arm — so without
    // these the error arms, the accessors and `Display` are dead regions the coverage gate counts
    // and no test would notice being deleted.

    #[test]
    fn canonical_form_rejects_an_empty_range() {
        let bad = [Range { start: 4, len: 0 }];
        let err = canonical_form(&bad, "law").expect_err("empty range must be rejected");
        assert_eq!(err.law(), "law");
        assert!(err.detail().contains("empty range at index 0"), "{err}");
    }

    #[test]
    fn canonical_form_rejects_adjacent_ranges() {
        // Touching, not overlapping: the case a byte-set comparison cannot see, and the one the
        // zero-length-claim defect in `subtract` produced.
        let bad = [Range { start: 0, len: 4 }, Range { start: 4, len: 4 }];
        let err = canonical_form(&bad, "law").expect_err("adjacent ranges must be rejected");
        assert!(err.detail().contains("adjacent or overlapping"), "{err}");
        assert_eq!(format!("{err}"), format!("law: {}", err.detail()));
    }

    #[test]
    fn canonical_form_rejects_descending_ranges() {
        let bad = [Range { start: 8, len: 2 }, Range { start: 0, len: 2 }];
        assert!(canonical_form(&bad, "law").is_err());
    }

    #[test]
    fn normalise_keeps_every_range_inside_the_universe() {
        for raw in [
            Range {
                start: u64::MAX,
                len: u64::MAX,
            },
            Range {
                start: UNIVERSE - 1,
                len: MAX_LEN,
            },
            Range { start: 0, len: 0 },
        ] {
            let r = normalise(raw);
            assert!(r.start < UNIVERSE && r.end() <= UNIVERSE, "{r:?}");
        }
        // Zero length must survive: it is the input class the claim filter turns on.
        assert_eq!(normalise(Range { start: 7, len: 0 }).len, 0);
    }
}
