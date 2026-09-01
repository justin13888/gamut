#!/usr/bin/env bash
# Mechanical conventions from `docs/testing.md` that a reader cannot be relied on to re-derive.
#
# Deliberately three rules, not a style guide. Each one encodes something the corpus pass (#435)
# established, each holds across the whole corpus *today*, and each fails for exactly one reason.
# A lint for a convention that does not yet hold is either noise or a wall, so anything the pass
# did not settle is not in here.
#
# What is deliberately NOT checked, and why:
#   * A tagged `//! <level> <technique> — ...` header. Only the files this pass rewrote carry the
#     tag; requiring it would fail ~120 files whose technique nobody has audited, and guessing the
#     tag for them would put a wrong claim in a header, which is worse than no claim.
#   * A filename vocabulary. There are ~100 distinct basenames under `crates/*/tests/`, and the
#     useful ones (`accounting`, `conformance`, `robustness`, `structure`) are already consistent
#     without being enumerable.
set -euo pipefail

fail=0
note() {
    printf '  %s\n' "$1"
}
problem() {
    printf '\ncheck-tests: %s\n' "$1"
    fail=1
}

# ---------------------------------------------------------------------------------------------
# 1. Every integration-test file carries a module doc.
#
# `docs/testing.md` asks a test file to name the one thing it pins. This is the floor: a file with
# no `//!` at all has not even tried. It holds for all 133 files today.
#
# The doc need not be the first line -- `gamut-color/tests/serde.rs` and its gamut-core twin lead
# with `#![cfg(feature = "serde")]`, which is the whole-file gate that legitimately puts them in
# `tests/` at all -- so the leading attribute/comment block is scanned rather than line 1.
# ---------------------------------------------------------------------------------------------
undocumented=()
while IFS= read -r f; do
    # The leading block: inner attributes, doc comments and blank lines, before any real item.
    if ! awk '/^#!\[/ || /^\/\/!/ || /^[[:space:]]*$/ { print; next } { exit }' "$f" |
        grep -q '^//!'; then
        undocumented+=("$f")
    fi
done < <(find crates -path 'crates/*/tests/*.rs' -not -path '*/tests/common/*' | sort)

if [ ${#undocumented[@]} -gt 0 ]; then
    problem "integration-test files with no module doc (what does this file pin?):"
    for f in "${undocumented[@]}"; do note "$f"; done
fi

# ---------------------------------------------------------------------------------------------
# 2. Every proptest configuration pins its seed and disables failure persistence.
#
# The highest-value rule here, and the reason this check exists at all. `docs/testing.md` calls
# determinism non-negotiable and puts it *in code*: the `--in-diff` mutation job is blocking, so a
# property whose pass/fail depends on OS entropy makes cargo-mutants report CAUGHT or MISSED for
# the same mutant on different runs -- and the equivalence proofs in `.cargo/mutants.toml` are only
# meaningful against a deterministic suite.
#
# `failure_persistence` is the half most likely to be forgotten. cargo-mutants reuses one tree copy
# across mutants, restoring only the file it mutated, so a `proptest-regressions` file written by
# one mutant's failure survives into the next mutant's run and is replayed first -- making
# CAUGHT/MISSED depend on mutant ordering and shard assignment.
# ---------------------------------------------------------------------------------------------
while IFS= read -r f; do
    grep -q 'proptest' "$f" || continue
    grep -qE 'Config \{|ProptestConfig' "$f" || continue

    if ! grep -q 'rng_seed: RngSeed::Fixed(' "$f"; then
        problem "proptest configuration without a pinned seed: $f"
        note "add  rng_seed: RngSeed::Fixed(0x...)  -- see docs/testing.md, \"Every property pins its seed\""
    fi
    if ! grep -q 'failure_persistence: None' "$f"; then
        problem "proptest configuration without failure_persistence disabled: $f"
        note "add  failure_persistence: None  -- a persisted regression file leaks between mutants"
    fi
done < <(find crates -name '*.rs' -path 'crates/*' | sort)

# ---------------------------------------------------------------------------------------------
# 3. Differential test files lead with `oracle`.
#
# Settled by the corpus pass: `oracle.rs` where a crate has one differential file,
# `oracle_<subject>.rs` where it has several. Leading with `oracle` groups a crate's differential
# files in a directory listing; the four `*_oracle.rs` stragglers were renamed in #451, #452 and
# #459. This rule is here only to stop the convention drifting back, and it is exact rather than
# a heuristic.
# ---------------------------------------------------------------------------------------------
stragglers=$(find crates -path 'crates/*/tests/*_oracle.rs' | sort || true)
if [ -n "$stragglers" ]; then
    problem "differential files should lead with 'oracle', not trail with it:"
    while IFS= read -r f; do
        note "$f  ->  $(dirname "$f")/oracle_$(basename "$f" | sed 's/_oracle\.rs$/.rs/')"
    done <<<"$stragglers"
fi

if [ "$fail" -ne 0 ]; then
    printf '\ncheck-tests: see docs/testing.md for the rules above.\n'
    exit 1
fi

printf 'check-tests: module docs, pinned proptest seeds and oracle filenames all conform\n'
