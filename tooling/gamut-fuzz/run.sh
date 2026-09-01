#!/usr/bin/env bash
# Runner for the dev-only fuzz tier (issues #264, #311), invoked as `mise run fuzz [target] [--]`.
#
# A script rather than an inline `run =` because mise appends a task's arguments to the end of its
# command, which a multi-line shell body cannot absorb -- the same reason `tooling/mutants/run.sh`
# exists.
set -euo pipefail

FUZZ_DIR="tooling/gamut-fuzz"

# cargo-fuzz 0.13 defaults `--target` to x86_64-unknown-linux-musl, and the sanitizer cannot link
# against a static libc ("sanitizer is incompatible with statically linked libc"). Pinning the host
# gnu triple is what makes the build reach a fuzz target at all.
target="$(rustc -vV | sed -n 's/^host: //p')"

if [ "$#" -eq 0 ]; then
    echo "fuzz targets (run one with: mise run fuzz <target>):"
    cargo +nightly fuzz list --fuzz-dir "$FUZZ_DIR"
    echo
    echo "  mise run fuzz ifd_read_ledger                        # until it finds something, or Ctrl-C"
    echo "  mise run fuzz ifd_read_ledger -- -max_total_time=60  # bounded"
    echo
    echo "A crash is minimised (cargo +nightly fuzz tmin --fuzz-dir $FUZZ_DIR <target> <input>)"
    echo "and then promoted into a NAMED DETERMINISTIC TEST in the crate's own suite."
    echo "The corpus is a search aid, not the regression record."
    exit 0
fi

exec cargo +nightly fuzz run --fuzz-dir "$FUZZ_DIR" --target "$target" "$@"
