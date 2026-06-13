# gamut task runner

# Format code
format:
    cargo fmt --all

# Check formatting without modifying
format-check:
    cargo fmt --all --check

# Lint with Clippy (warnings as errors)
lint:
    cargo clippy --workspace --all-targets --all-features -- -D warnings

# Lint and auto-fix
lint-fix:
    cargo clippy --workspace --fix --allow-dirty --allow-staged

# Check this branch's commits follow Conventional Commits (convco)
check-commits:
    convco check "$(git merge-base origin/master HEAD)..HEAD"

# Install the gamut CLI binary
install-cli:
    cargo install --path crates/gamut-cli

# Run tests
test:
    cargo test --workspace --all-features

# Run performance benchmarks (Divan; issue #149). Like `just test`, this builds the codec
# crates' cross-check-oracle dev-dependencies, so it needs the submodules + C toolchain.
# To pass Divan flags (e.g. `--sample-count 50`) target one harness directly, since the
# per-crate libtest stubs reject them: `cargo bench -p gamut-webp --bench codec -- <flags>`.
bench:
    cargo bench --workspace

# Run tests with coverage (enforces 80% line coverage).
# Bindings/binary crates (cli, wasm, ffi) and the dev-only `tooling/` oracles are
# excluded — their entry points are not meaningfully unit-testable and would otherwise
# skew the gate.
coverage:
    cargo llvm-cov --workspace --all-features --ignore-filename-regex '(crates/gamut-(cli|wasm|ffi)|tooling)/' --fail-under-lines 80

# Mutation testing, whole workspace; slow. Needs `mise install` + submodules + C toolchain.
mutants:
    cargo mutants

# Mutation testing of only the code changed vs master; fast (mirrors the PR CI job).
mutants-diff:
    git diff origin/master...HEAD > target/mutants.diff
    cargo mutants --in-diff target/mutants.diff

# Mutation testing of one crate, e.g. `just mutants-crate gamut-bitstream`.
mutants-crate crate:
    cargo mutants -p {{crate}}

# List every workspace crate and its version
versions:
    cargo metadata --no-deps --format-version 1 | jq -r '.packages | sort_by(.name)[] | "\(.name) \(.version)"'

# Bump a single crate's version (level = major | minor | patch). Convenience/escape
# hatch; routine bumps are automated by release-plz from conventional commits.
bump crate level:
    cargo set-version -p {{crate}} --bump {{level}}
