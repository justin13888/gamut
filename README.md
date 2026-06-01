# gamut

A collection of space-efficient image encoding libraries.

## Prerequisites

- [Rust (rustup)](https://rustup.rs) -- toolchain (channel pinned via `rust-toolchain.toml`)
- [just](https://github.com/casey/just) -- command runner
- [Lefthook](https://github.com/evilmartians/lefthook) -- git hooks manager
- [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov) -- code coverage tool

## Quick Start

```bash
cargo build
cargo test
```

## Development

| Command          | Description                          |
| ---------------- | ------------------------------------ |
| `cargo build`    | Build the crate                      |
| `just test`      | Run tests                            |
| `just format`    | Format code                          |
| `just lint`      | Lint with Clippy (warnings as errors)|
| `just lint-fix`  | Lint and auto-fix                    |
| `just coverage`  | Run tests with coverage (min 80%)    |

## Tech Stack

- **Language:** Rust (edition 2024)
- **Formatter:** rustfmt
- **Linter:** Clippy
- **Key Dependencies:** tracing, thiserror

## Git Hooks

This project uses [Lefthook](https://github.com/evilmartians/lefthook). Pre-commit hooks
auto-fix formatting and linting on staged files. Pre-push hooks run format checks, lint
checks, tests, and a coverage gate.

## CI/CD

GitHub Actions runs format checks, linting, tests, and coverage on pushes to `master` and
pull requests.

## Code Coverage

This project uses [`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov) for
LLVM-based code coverage. CI enforces a minimum of 80% line coverage.

```bash
just coverage
```

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.
