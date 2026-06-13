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

# Run tests
test:
    cargo test --workspace --all-features

# Run performance benchmarks
bench:
    cargo bench --workspace --all-features

# Run tests with coverage (enforces 80% line coverage).
# Bindings/binary crates (cli, wasm, ffi) are excluded — their entry points are not
# meaningfully unit-testable and would otherwise skew the gate.
coverage:
    cargo llvm-cov --workspace --all-features --ignore-filename-regex 'crates/gamut-(cli|wasm|ffi)/' --fail-under-lines 80
