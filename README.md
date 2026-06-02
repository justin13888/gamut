# gamut

A collection of space-efficient image encoding libraries, organized as a Cargo workspace.

## Why gamut?

The world doesn't lack image codecs. libavif/libaom, libwebp, and libjpeg-turbo are
mature, fast, and battle-tested — we're not out to beat a decade of hand-tuned SIMD
assembly on raw encode speed. gamut exists because "fast C that works" still leaves real
gaps, and those gaps are exactly where a clean-slate, pure-Rust, permissively-licensed
implementation wins.

- **Memory safety on the industry's worst attack surface.** Image parsers chew on hostile,
  attacker-controlled bytes from the open internet, and the C codecs have the CVE record to
  show how that goes — libwebp's CVE-2023-4863 was a zero-click, wormable heap overflow that
  triggered emergency out-of-band patches across browsers, Electron apps, and mobile OSes in
  a single week. Safe Rust deletes that entire bug class (spatial and temporal memory
  corruption) from the encode and parse paths. For anything that ingests untrusted images,
  that alone justifies the rewrite.

- **Builds anywhere `cargo` does.** No autotools, no CMake, no nasm/yasm, no vendored C, no
  FFI boundary to audit. `cargo build` cross-compiles cleanly to wasm32, aarch64, and musl
  targets that libaom makes miserable — one toolchain, reproducible builds, no system-library
  version skew.

- **WASM as a first-class target, not an afterthought.** The C codecs run through Emscripten
  come out large, slow to instantiate, and awkward to tree-shake. A native Rust → wasm build
  is smaller and talks to the JS/TS ecosystem directly, which makes serverless/edge image
  optimization (Workers, Lambda, and friends) practical instead of shipping a multi-megabyte
  blob.

- **A genuinely clean license story.** gamut deliberately targets royalty-free formats and
  ships under MIT OR Apache-2.0 — no GPL/LGPL reach, no vendored-code license soup, no
  static-linking exceptions to reason about. Patent-unencumbered formats deserve
  permissively-licensed code to match.

- **Encoder-first, size-first — the gap the Rust ecosystem actually has.** Most Rust imaging
  is decode-only and hands the hard encoders off to C wrappers. gamut is built the other way
  round: encoders are the product, and the thing we optimize is *output bytes at a given
  quality and speed*, with the space/time tradeoff documented per format. That's the number
  that lands on storage and bandwidth bills.

- **One codebase, shared primitives.** Color management, DSP, bitstream, and container parsing
  live in shared crates (`gamut-color`, `gamut-dsp`, `gamut-bitstream`, `gamut-isobmff`,
  `gamut-riff`) instead of being re-implemented inside each separate C library. Consistent
  behavior across formats, one API, one place to fix a color bug — and you compile in only the
  formats you enable via Cargo features.

- **Readable enough to change.** Implemented clean-slate from the official specs in
  `references/`, the code is something you can actually audit, fork, and experiment with —
  not decades of accreted platform `#ifdef`s and inline assembly.

### Scope

The initial focus is **AVIF, WebP, and JPEG** — the formats with the best
size-versus-compatibility tradeoff today. JPEG XL is intentionally out of scope for now (it
is better served by a dedicated effort). The other format crates in the tree (HEIC, VVC,
AV2, JXL) are scaffolding, and may move or be dropped as the focus sharpens.

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

## Releases

Publishing to crates.io is automated with [release-plz](https://release-plz.dev). On pushes
to `master` it opens a release PR (version bumps + changelogs); merging that PR publishes
every changed crate in dependency order. Requires a `CARGO_REGISTRY_TOKEN` repository secret.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.
