# gamut-fuzz

The dev-only fuzz tier (issues #264, #311). It drives the crates' `invariants` modules — the
**same** executable laws the pinned-seed `proptest` properties drive in the per-PR gate — under
libFuzzer.

```bash
mise run fuzz                                        # list the targets
mise run fuzz ifd_read_ledger                        # until it finds something, or Ctrl-C
mise run fuzz ifd_read_ledger -- -max_total_time=60  # bounded
```

## Why the law is not restated here

`docs/testing.md` puts it as *"a property is the specification a fuzzer checks"*. Each law is a
plain function in its crate's `invariants` module, exposed by the `doc(hidden)` `test-support`
feature, and **both** tiers call it. A law written twice is a law that can disagree with itself,
and the disagreement would be invisible: each tier would keep passing against its own copy.

What differs between the tiers is only the search:

| | property (per-PR gate) | this tier (extended CI) |
|---|---|---|
| cases | 512, from a pinned seed | unbounded, coverage-guided |
| reproducible | yes, by construction | only via a saved input |
| runs in | `coverage`, the blocking gate | explicitly, or `extended.yml` |

The pilot in #434 is the argument that the second tier earns its place: the property there killed
a mutant `.cargo/mutants.toml` had recorded as **provably equivalent**, on an input no hand-written
test had thought to try. 512 cases found it; this tier searches for the ones 512 will not.

## Why it is not in the per-PR gate

`coverage` is the only CI job that runs tests, so anything in it must be bounded and reproducible.
A coverage-guided engine is neither — it is time-bounded rather than iteration-bounded, and under
`cargo llvm-cov` instrumentation it would explore an unrecorded, load-dependent number of inputs.
That would be strictly *weaker* than the exhaustive truncation and single-byte-overwrite sweeps
already in `gamut-ifd/tests/robustness.rs`, while making the blocking mutation job
non-deterministic.

So it sits where the DNG sample corpus sits: an excluded `tooling/` crate, invoked explicitly.

## What to do with a crash

**Minimise it, then promote it into a named deterministic test** in the crate's own suite:

```bash
cargo +nightly fuzz tmin --fuzz-dir tooling/gamut-fuzz ifd_read_ledger <input>
```

The corpus is a search aid, not the regression record. A saved input is only reproducible while
the target's byte-to-input mapping is unchanged; a named case is reproducible forever. This is the
same rule `docs/testing.md` applies to a shrunk `proptest` counterexample, and the reason
`failure_persistence` is off there.

## Notes

- **Nightly is required.** libFuzzer needs `-Z sanitizer=fuzzer`, which stable does not expose.
  `rust-toolchain.toml` pins stable; the runner selects nightly for this crate only, the same shape
  as `mise run fmt` selecting a nightly rustfmt for its nightly-only options.
- **The host target is pinned.** cargo-fuzz 0.13 defaults `--target` to
  `x86_64-unknown-linux-musl`, and the sanitizer cannot link against a static libc. Without the
  pin the build fails before reaching a target at all.
- **This crate is workspace-excluded**, so `cargo test --workspace --all-features` never builds it.
- **The `--` is reconstructed, not passed through.** mise swallows a task's `--`, so
  `mise run fuzz t -- -max_total_time=60` reaches `run.sh` as two bare words and cargo-fuzz would
  reject the second as one of its own options. The runner re-splits on libFuzzer's own flag syntax
  (`-name=value`, single dash plus an `=`), so libFuzzer arguments and cargo-fuzz options both land
  where they belong with or without the separator. An explicit `--` still wins outright.

## Targets

| target | crate | laws |
|---|---|---|
| `ifd_read_ledger` | `gamut-ifd` | `ledger_is_canonical`, `subtract_is_set_difference` |

`gamut-core` and `gamut-tonemap` gain `invariants` modules in #450; their targets are added here
when that merges. The structure is deliberately one file per crate, so that is an additive change.
