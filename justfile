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

# Install the gamut CLI binary
install-cli:
    cargo install --path crates/gamut-cli

# Run tests
test:
    cargo test --workspace --all-features

# Run tests with coverage (enforces 80% line coverage).
# Bindings/binary crates (cli, wasm, ffi) and the dev-only `tooling/` oracles are
# excluded — their entry points are not meaningfully unit-testable and would otherwise
# skew the gate.
coverage:
    cargo llvm-cov --workspace --all-features --ignore-filename-regex '(crates/gamut-(cli|wasm|ffi)|tooling)/' --fail-under-lines 80

# List every workspace crate and its version
versions:
    cargo metadata --no-deps --format-version 1 | jq -r '.packages | sort_by(.name)[] | "\(.name) \(.version)"'

# Bump a single crate's version (level = major | minor | patch). Convenience/escape
# hatch; routine bumps are automated by release-plz from conventional commits.
bump crate level:
    cargo set-version -p {{crate}} --bump {{level}}
