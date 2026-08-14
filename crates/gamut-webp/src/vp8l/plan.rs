//! The VP8L encoder's candidate-plan ladder (issue #31).
//!
//! Every knob the encoder can turn lives in a [`Vp8lPlan`], so encoding under a plan is a pure,
//! deterministic function of `(pixels, dimensions, plan)`. [`enumerate`] then maps an [`Effort`]
//! onto the list of plans to try, and the driver in [`super::encoder`] encodes each one and keeps
//! the shortest.
//!
//! # Why the ladder is monotone
//!
//! [`enumerate`] is **append-only**: `enumerate(e - 1)` is a prefix of `enumerate(e)`, built by
//! extending the previous rung's list rather than replacing it. The driver keeps the shortest
//! encoding and breaks ties toward the earlier plan, so a level-`e` result is the minimum over a
//! superset of the level-`e-1` candidates. Output size is therefore non-increasing in effort **for
//! every image, by construction** rather than by measurement — and equal whenever nothing new
//! helps, which keeps the upper rungs byte-identical on content they cannot improve.
//!
//! This is why the search is one flat list of *complete* plans rather than two stages ("pick a
//! transform chain, then refine the parse"). A staged search can have its stage-one winner lose
//! under the refined parse, which would break the nesting the guarantee rests on.

use crate::config::Effort;

/// The transform chain a plan emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Structure {
    /// Take the palette path when the image has few enough distinct colours, else the spatial path
    /// with the full transform chain. The rung-0 spine, and the encoder's historical behaviour.
    Auto,
}

/// How many bits of colour cache a plan uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CacheBits {
    /// The size heuristic applied to the image actually being coded.
    Auto,
}

/// How a plan splits the image into prefix-code groups.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Grouping {
    /// Group meta-blocks by their most frequent green symbol.
    Signature {
        /// Block-size exponent for the entropy image.
        prefix_bits: u32,
    },
}

/// The LZ77 match-finder's search budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Lz77Params {
    /// Maximum hash-chain length walked per position.
    pub max_chain: usize,
}

/// One complete VP8L encoding configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Vp8lPlan {
    /// The transform chain to emit.
    pub structure: Structure,
    /// The colour-cache size to use.
    pub cache: CacheBits,
    /// How to split the image into prefix-code groups.
    pub grouping: Grouping,
    /// The LZ77 search budget.
    pub lz77: Lz77Params,
}

/// The rung-0 spine: the single plan every higher rung's candidate list starts from.
const SPINE: Vp8lPlan = Vp8lPlan {
    structure: Structure::Auto,
    cache: CacheBits::Auto,
    grouping: Grouping::Signature { prefix_bits: 4 },
    lz77: Lz77Params { max_chain: 32 },
};

/// Hard ceiling on the number of candidates each rung may enumerate, so encode cost cannot grow
/// silently as the ladder fills in.
pub(crate) const MAX_PLANS: [usize; 7] = [1, 3, 6, 10, 15, 22, 30];

/// The candidate plans for `effort`, in evaluation order.
///
/// Append-only by construction: each rung extends the previous rung's list, which is what makes
/// output size non-increasing in effort (see the module docs).
#[must_use]
pub(crate) fn enumerate(effort: Effort) -> Vec<Vp8lPlan> {
    let mut plans = vec![SPINE];
    for level in 1..=effort.level() {
        plans.extend(added_at(level));
    }
    debug_assert!(
        plans.len() <= MAX_PLANS[effort.level() as usize],
        "effort {} enumerated {} plans, over its ceiling",
        effort.level(),
        plans.len()
    );
    plans
}

/// The plans rung `level` adds on top of rung `level - 1`.
///
/// Empty for now at every rung: the ladder's rungs are filled in by the optimizations that follow,
/// and until then every effort level selects the spine and produces identical output.
fn added_at(_level: u8) -> Vec<Vp8lPlan> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_rung_extends_the_one_below_it() {
        // The monotonicity guarantee rests entirely on this: a rung's candidate list must be a
        // prefix-extension of the rung below, never a replacement. If someone converts `added_at`
        // into a "choose a different set per level" table, this fails.
        for level in 1..=6u8 {
            let lower = enumerate(Effort::from_level(level - 1).expect("in range"));
            let upper = enumerate(Effort::from_level(level).expect("in range"));
            assert!(
                upper.len() >= lower.len(),
                "rung {level} shrank the candidate list"
            );
            assert_eq!(
                &upper[..lower.len()],
                &lower[..],
                "rung {level} is not an extension of rung {}",
                level - 1
            );
        }
    }

    #[test]
    fn every_rung_stays_within_its_candidate_ceiling() {
        // The ceiling is the encode-cost contract; enumerating past it would make a rung
        // arbitrarily slow without anyone noticing.
        for level in 0..=6u8 {
            let plans = enumerate(Effort::from_level(level).expect("in range"));
            assert!(
                !plans.is_empty(),
                "rung {level} must offer at least one plan"
            );
            assert!(
                plans.len() <= MAX_PLANS[level as usize],
                "rung {level} enumerated {} plans, ceiling {}",
                plans.len(),
                MAX_PLANS[level as usize]
            );
        }
    }

    #[test]
    fn the_spine_is_every_rungs_first_candidate() {
        // Ties resolve to the earliest plan, so the spine being first is what makes an unhelpful
        // higher rung reproduce the lower rung's bytes exactly rather than merely its size.
        for level in 0..=6u8 {
            let plans = enumerate(Effort::from_level(level).expect("in range"));
            assert_eq!(plans[0], SPINE, "rung {level} does not lead with the spine");
        }
    }
}
