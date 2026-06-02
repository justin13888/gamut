# gamut

A collection of space-efficient image encoding libraries, organized as a Cargo workspace.

## Usage

Add the umbrella `gamut` crate and enable only the formats you need:

```toml
[dependencies]
gamut = { version = "0.1", features = ["avif", "jxl"] }
```

The umbrella has no default features, so a bare dependency compiles only `gamut-core`.

## Crates

| Crate             | Purpose                                                                 | Status      |
| ----------------- | ----------------------------------------------------------------------- | ----------- |
| `gamut`           | Umbrella crate; re-exports the format crates behind Cargo features      | scaffold    |
| `gamut-core`      | Core traits (`Encoder`/`Decoder`), image buffers, dimensions, errors    | scaffold    |
| `gamut-color`     | Color spaces, pixel formats, bit depths, chroma subsampling, transfers  | placeholder |
| `gamut-dsp`       | Shared DSP: DCT, wavelet transforms, quantization, filtering            | placeholder |
| `gamut-bitstream` | Bit readers/writers and entropy coders (ANS, arithmetic, Huffman)       | placeholder |
| `gamut-isobmff`   | ISOBMFF container utilities (AVIF, HEIC)                                 | placeholder |
| `gamut-riff`      | RIFF container utilities (WebP)                                          | placeholder |
| `gamut-av1`       | AV1 image encoder/decoder (basis for AVIF)                              | placeholder |
| `gamut-av2`       | AV2 (next-gen AV1 successor) encoder/decoder                            | placeholder |
| `gamut-avif`      | AVIF encoder/decoder                                                     | placeholder |
| `gamut-jxl`       | JPEG XL encoder/decoder                                                  | placeholder |
| `gamut-webp`      | WebP encoder/decoder                                                     | placeholder |
| `gamut-heic`      | HEIC/HEIF encoder/decoder                                                | placeholder |
| `gamut-vvc`       | VVC (H.266) encoder/decoder                                              | placeholder |
| `gamut-cli`       | `gamut` command-line image converter                                    | placeholder |
| `gamut-wasm`      | WebAssembly bindings                                                     | placeholder |
| `gamut-ffi`       | C-compatible FFI bindings                                                | placeholder |

All cargo metadata is centralized in the root `[workspace.package]` /
`[workspace.dependencies]`; each crate inherits via `.workspace = true`.

## Prerequisites

- [Rust (rustup)](https://rustup.rs) -- toolchain (channel pinned via `rust-toolchain.toml`)
- [just](https://github.com/casey/just) -- command runner
- [Lefthook](https://github.com/evilmartians/lefthook) -- git hooks manager
- [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov) -- code coverage tool

## Quick Start

```bash
cargo build --workspace
cargo test --workspace
```

## Development

| Command          | Description                              |
| ---------------- | ---------------------------------------- |
| `cargo build --workspace` | Build all crates                |
| `just test`      | Run tests (workspace, all features)      |
| `just format`    | Format code                              |
| `just lint`      | Lint with Clippy (warnings as errors)    |
| `just lint-fix`  | Lint and auto-fix                        |
| `just coverage`  | Run tests with coverage (min 80%)        |

## Tech Stack

- **Language:** Rust (edition 2024)
- **Formatter:** rustfmt
- **Linter:** Clippy
- **Release:** [release-plz](https://release-plz.dev) (dependency-ordered crates.io publishing)
- **Key Dependencies:** tracing, thiserror, bitflags, clap, wasm-bindgen

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

The bindings/binary crates (`gamut-cli`, `gamut-wasm`, `gamut-ffi`) are excluded from the
gate — their entry points are not meaningfully unit-testable.

## AI Policy

Vibe-coded contributions are welcome. AI-assisted PRs are accepted as long as you
personally vouch for the work — you've read it, you understand it, and you stand behind it
as if you'd written every line — and it matches the project's existing code style and
requirements. The CI and git hooks loosely enforce the bare minimum; meeting that bar is
necessary but not sufficient. Review your output before opening a PR.

## Versioning

The workspace ships a single unified version: every crate shares `version` from
`[workspace.package]` and is bumped together. On a minor release, all crates move to the new
minor version even if a given crate saw no meaningful change — keeping the version numbers
aligned across the workspace is worth more than per-crate precision.

Patch versions are the exception: a hot patch may be released for individual crates, so patch
levels can diverge between releases before the next minor pulls everything back into lockstep.

## Releases

Publishing to crates.io is automated with [release-plz](https://release-plz.dev). On pushes
to `master` it opens a release PR (version bumps + changelogs); merging that PR publishes
every changed crate in dependency order. Requires a `CARGO_REGISTRY_TOKEN` repository secret.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.
