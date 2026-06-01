# gamut task runner

# Format code
format:
    cargo fmt

# Check formatting without modifying
format-check:
    cargo fmt --check

# Lint with Clippy (warnings as errors)
lint:
    cargo clippy -- -D warnings

# Lint and auto-fix
lint-fix:
    cargo clippy --fix --allow-dirty --allow-staged

# Run tests
test:
    cargo test

# Run tests with coverage (enforces 80% line coverage)
coverage:
    cargo llvm-cov --fail-under-lines 80
