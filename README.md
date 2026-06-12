# gamut

> Project Status: Early development. Do not use it for anything serious!

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
  that lands on storage and bandwidth bills. Decoders may follow where the Rust ecosystem
  lacks a strong, feature-complete implementation, but encoders are the priority.

- **One codebase, shared primitives.** Color management, DSP, bitstream, and container parsing
  live in shared crates (`gamut-color`, `gamut-dsp`, `gamut-bitstream`, `gamut-isobmff`,
  `gamut-riff`) instead of being re-implemented inside each separate C library. Consistent
  behavior across formats, one API, one place to fix a color bug — and you compile in only the
  formats you enable via Cargo features.

- **Readable enough to change.** Implemented clean-slate from the official specs in
  `references/`, the code is something you can actually audit, fork, and experiment with —
  not decades of accreted platform `#ifdef`s and inline assembly.

### Author's Remarks

In 2026, `gamut` started when there were no robust, well-tested Rust implementations of various image and color primitives. We want this to be the de-facto, permissivel-licensed choice for most color and image needs, primarily for professional use cases. Implementing this ecosystem of libraries (crates) without commercial backing is also made possible when image formats are spec-driven and LLM agents sufficiently speed up work (when used correctly).

### Scope

The initial focus is **AVIF, WebP, and JPEG** — the formats with the best
size-versus-compatibility tradeoff today. JPEG XL is intentionally out of scope for now (it
is better served by a dedicated effort). The other format crates in the tree (HEIC, VVC,
AV2, JXL) are scaffolding, and may move or be dropped as the focus sharpens.

Alongside the codecs, gamut is growing **shared image-metadata primitives** (issue #34) — EXIF,
XMP, ICC, and IPTC, plus the TIFF/IFD container core (`gamut-ifd`) that EXIF builds on — so the
format crates can read, preserve, and embed metadata. These are newly scaffolded; the long-term
goal is de-facto, fully-featured implementations (for EXIF, exiftool-class tag coverage).

**gamut is image-first.** Even where a format's codec (AV1, AV2, VVC, HEVC) is fundamentally a
video codec, gamut implements only the intra-frame, still-image subset those formats use — no
inter-frame prediction, no motion compensation, no video sequences. The video-named codec
crates (`gamut-av1`, `gamut-av2`, `gamut-vvc`, and HEVC-based `gamut-heic`) are still-image
encoders, not video codecs, and gamut will not grow video primitives. This extends to
container-level multi-frame sequences: WebP animation (`ANIM`/`ANMF`) is out of scope even
though each frame is an independent keyframe — only single still images are supported.

## Usage

Add the umbrella `gamut` crate and enable only the formats you need:

```toml
[dependencies]
gamut = { version = "0.1", features = ["avif", "jxl"] }
```

The umbrella has no default features, so a bare dependency compiles only `gamut-core`. The
`primitives` feature additionally re-exports the shared building blocks (`gamut::color` /
`gamut::dsp` / `gamut::bitstream`) for tooling and sandbox use; `all` enables it along with every
format.

## Crates

| Crate             | Purpose                                                                | Status                                 |
| ----------------- | ---------------------------------------------------------------------- | -------------------------------------- |
| `gamut`           | Umbrella crate; re-exports the format crates behind Cargo features     | implemented                            |
| `gamut-core`      | Core traits (`Encoder`/`Decoder`), image buffers, dimensions, errors   | WIP                                    |
| `gamut-color`     | Color spaces, pixel formats, bit depths, chroma subsampling, transfers | stabilizing api                        |
| `gamut-dsp`       | Shared DSP: DCT, wavelet transforms, quantization, filtering           | stabilizing api                        |
| `gamut-bitstream` | Bit readers/writers and entropy coders (ANS, arithmetic, Huffman)      | stabilizing api                        |
| `gamut-isobmff`   | ISOBMFF container utilities (AVIF, HEIC)                               | finalizing api                         |
| `gamut-riff`      | RIFF container utilities (WebP)                                        | finalizing api                         |
| `gamut-av1`       | AV1 still-image (intra-frame) encoder — the codec layer beneath AVIF   | implemented lossless and lossy (alpha) |
| `gamut-av2`       | AV2 still-image (intra-frame) encoder/decoder — AV1's successor        | placeholder                            |
| `gamut-avif`      | AVIF encoder — AV1 still frames in an ISOBMFF container                | stabilizing with gamut-av1             |
| `gamut-jxl`       | JPEG XL encoder/decoder                                                | placeholder                            |
| `gamut-webp`      | WebP (intra-frame VP8/VP8L) encoder/decoder                            | implemented VP8 + VP8L (+alpha)        |
| `gamut-heic`      | HEIC/HEIF still-image (HEVC intra) encoder/decoder                     | placeholder                            |
| `gamut-vvc`       | VVC (H.266) still-image (intra) encoder/decoder                        | placeholder                            |
| `gamut-ifd`       | TIFF/IFD container core (byte order, field types, IFD I/O) — EXIF+TIFF | scaffolding (impl in progress, #34)    |
| `gamut-cli`       | `gamut` CLI sandbox: encode AVIF + inspect the shared primitives       | ready for use                          |
| `gamut-wasm`      | WebAssembly bindings                                                   | placeholder                            |
| `gamut-ffi`       | C-compatible FFI bindings                                              | placeholder                            |

All cargo metadata except per-crate `version` is centralized in the root
`[workspace.package]` / `[workspace.dependencies]`; each crate inherits the shared fields via
`.workspace = true` and sets its own `version` (see [Versioning](#versioning)).

## Prerequisites

- [Rust (rustup)](https://rustup.rs) -- toolchain (channel pinned via `rust-toolchain.toml`);
  see [Minimum Supported Rust Version](#minimum-supported-rust-version) for the lower bound
- [mise](https://mise.jdx.dev) -- provisions the rest of the dev tooling from `mise.toml`:
  [just](https://github.com/casey/just) (command runner),
  [Lefthook](https://github.com/evilmartians/lefthook) (git hooks),
  [convco](https://convco.github.io) (conventional-commit linter),
  [jq](https://jqlang.github.io/jq/),
  [cargo-llvm-cov](https://github.com/taiki-e/cargo-llvm-cov) (coverage),
  [cargo-edit](https://github.com/killercup/cargo-edit) (`cargo set-version` for `just bump`),
  and the C build tools [CMake](https://cmake.org), [Ninja](https://ninja-build.org) and
  [Meson](https://mesonbuild.com). After cloning, run `mise trust && mise install`, then
  activate mise in your shell (e.g. `eval "$(mise activate zsh)"`) so these land on `PATH` —
  the git hooks and `just` recipes invoke them directly.

Building the **shipped crates** needs only the Rust toolchain — they are pure Rust with no C
dependencies. Building the **cross-check tests** additionally needs a C toolchain plus
[nasm](https://www.nasm.us) and pkg-config — the two build deps that stay system packages
(CMake, Ninja and Meson come from mise). Those tests link reference decoders (dav1d, libavif)
built from the git submodules under `third_party/` via the dev-only oracle crates in `tooling/`;
nothing is taken from system-installed decoders. Install the two on Debian/Ubuntu with
`sudo apt-get install nasm pkg-config` (macOS: `brew install nasm pkg-config`).

## Quick Start

```bash
# The cross-check tests link vendored dav1d/libavif from third_party/ submodules.
git submodule update --init --recursive

# Dev tooling + git hooks (see Prerequisites; also needs system nasm + pkg-config).
mise trust && mise install
lefthook install

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
| `just check-commits` | Check commits are Conventional Commits |
| `just coverage`  | Run tests with coverage (min 80%)        |
| `just versions`  | List every crate's version               |
| `just bump <crate> <level>` | Bump one crate (`major`\|`minor`\|`patch`) |

## Minimum Supported Rust Version (MSRV)

The MSRV is **Rust 1.88** (stable), built against **edition 2024**. This is the lowest
version CI is expected to support, and it is declared once in the root `[workspace.package]`
(`rust-version = "1.88"`); every crate inherits it via `rust-version.workspace = true`.

Policy:

- The MSRV is the floor we test and publish against, not necessarily the newest toolchain.
  Day-to-day development tracks the latest `stable` (pinned to the `stable` channel in
  `rust-toolchain.toml`).
- Raising the MSRV is a deliberate, semver-relevant change: bump `rust-version` in the root
  `Cargo.toml` and note it here. Pre-1.0, an MSRV bump rides a minor release.
- Edition (`2024`) is likewise centralized in `[workspace.package]` and inherited by every
  crate; it changes only alongside an MSRV bump that allows it.

## Git Hooks

This project uses [Lefthook](https://github.com/evilmartians/lefthook) (provisioned by mise);
run `lefthook install` once after `mise install` to register the hooks. The `commit-msg` hook
rejects messages that aren't [Conventional Commits](https://www.conventionalcommits.org)
(policy in `.convco`). Pre-commit hooks auto-fix formatting and linting on staged files.
Pre-push hooks re-check the branch's commit messages and run format checks, lint checks, tests,
and a coverage gate. The hooks call mise-managed tools (convco, cargo-llvm-cov), so keep mise
activated in your shell.

## CI/CD

GitHub Actions provisions tooling via [mise](https://mise.jdx.dev) and runs format checks,
linting, tests, and coverage on pushes to `master` and pull requests. Pull requests
additionally validate every commit message against Conventional Commits with convco.

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

Every crate is versioned **independently** following [SemVer](https://semver.org), based on
its own changes. There is **no** guarantee that versions line up across the workspace — a
change to one codec bumps only that crate (and anything that depends on it), so version
numbers drift apart over time. Only `version` is per-crate; shared metadata such as the
edition and [MSRV](#minimum-supported-rust-version) stays workspace-owned.

Bumps are automated: [release-plz](https://release-plz.dev) reads each crate's
conventional-commit history, computes its next version, and updates dependents' requirements
as needed. Each crate keeps its own `CHANGELOG.md` and is tagged and GitHub-released as
`<crate>-v<version>` (e.g. `gamut-core-v0.2.0`) — there is no single repo-wide version tag,
so the umbrella `gamut` crate's version serves as the headline "project" version. Run
`just versions` to see every crate's current version at a glance.

Because release-plz keys versions and changelogs off commit messages, those messages are
enforced as [Conventional Commits](https://www.conventionalcommits.org) — by the git hooks
locally and the CI PR check (see [Git Hooks](#git-hooks)) — to keep the changelogs clean.

## Releases

Publishing to crates.io is automated with [release-plz](https://release-plz.dev). On pushes
to `master` it opens a release PR (per-crate version bumps + changelogs); merging that PR
publishes every changed crate in dependency order, then creates the per-crate tags and GitHub
releases. Publishing authenticates via crates.io
[Trusted Publishing](https://crates.io/docs/trusted-publishing) (OIDC) — no
`CARGO_REGISTRY_TOKEN` secret is stored.

## License

Licensed under either of [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE) at your option.
