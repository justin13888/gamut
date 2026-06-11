# gamut

A collection of space-efficient image encoding libraries, organized as a Cargo workspace
under `crates/`.

## Workspace layout

The umbrella crate `gamut` re-exports format crates behind Cargo features; everything builds
on shared primitives.

gamut is **image-first** and implements no video primitives: codec crates named after video
formats (`av1`/`av2`/`vvc`, HEVC-based `heic`, VP8-lineage `webp`) cover only the intra-frame
still-image subset — no inter-frame/motion/sequence coding. Encoder-first; decoders only where
the Rust ecosystem lacks a strong, feature-complete implementation.

Dependency edges (a crate depends on those to its right):

- **gamut** -- umbrella; optional deps on the format crates, gated by features (`avif`,
  `jxl`, `webp`, `heic`, `vvc`, `av1`, `av2`, `all`). `default = []`. The `primitives` feature
  additionally re-exports the shared `color`/`dsp`/`bitstream` crates for tooling; `all` includes it.
- **gamut-core** -- `Encoder`/`Decoder` traits, image buffers, `Dimensions`, `Error`. No
  internal deps; everything else depends on it.
- **gamut-color** / **gamut-dsp** / **gamut-bitstream** -- shared primitives. ← core.
- **gamut-isobmff** (AVIF/HEIC container) / **gamut-riff** (WebP container). ← core, bitstream.
- **gamut-av1** / **gamut-av2** / **gamut-jxl** / **gamut-vvc** -- codecs. ← core, color, dsp, bitstream.
- **gamut-avif** ← av1, isobmff, core, color. **gamut-webp** ← +riff. **gamut-heic** ← isobmff, core, color.
- **gamut-cli** (binary named `gamut`) / **gamut-wasm** (cdylib) / **gamut-ffi** (cdylib/staticlib). ← gamut.
  `gamut-cli` is the sandbox that exercises the implemented features: it decodes input via the
  third-party `image` crate (PNG/JPEG/PPM) but encodes only with gamut crates, and exposes the
  `primitives` re-exports as inspection subcommands.

## Reference

All codec implementations must follow the official specs that should be attached in `references/`

## Validation

Dev tooling (just, lefthook, convco, cargo-llvm-cov, CMake/Ninja/Meson, …) is provisioned by
[mise](https://mise.jdx.dev): run `mise install` and activate mise in your shell. Validate
changes:

```bash
just test            # correctness
just format-check    # formatting
just lint            # lint (Clippy, warnings as errors)
just coverage        # coverage (minimum 80%)
just check-commits   # commit messages are Conventional Commits
```

The shipped crates are pure Rust, but the decoder cross-check tests link reference decoders
(dav1d, libavif) built from the `third_party/` git submodules via the dev-only oracle crates in
`tooling/`. Running the tests therefore needs the submodules checked out
(`git submodule update --init --recursive`) and the build tools on `PATH` — CMake/Ninja/Meson
come from mise; nasm and pkg-config are system packages (`apt-get install nasm pkg-config`).
No system-installed decoder binaries are used.

## Conventions

- All `pub` items need doc comments. Mark fallible/owning return types with `#[must_use]`
  where dropping the value is likely a bug.
- No `unwrap()`/`expect()` in library code paths — return typed errors via `thiserror`.
- Keep encoders allocation-conscious: prefer slices and `&[u8]` over owned buffers in hot
  paths, and document the space/time tradeoff of each format.
- Stub crates must stay region-free for the coverage gate: a placeholder `lib.rs` holds only
  module docs + declarations (traits/types without bodies), **no placeholder `fn` bodies**
  (a `todo!()`-bodied fn adds an uncovered region). The `gamut-(cli|wasm|ffi)` crates are
  excluded from coverage via `--ignore-filename-regex`.

## Versioning

Each crate is versioned **independently** per SemVer — there is no shared workspace version,
and releases do not guarantee version consistency across crates. Only `version` is per-crate;
all other metadata (`edition`, `rust-version`/MSRV, license, repository) is workspace-owned
and inherited via `*.workspace = true`. Version bumps, per-crate changelogs, and crates.io
publishing are automated by release-plz from conventional-commit history — write conventional
commit messages (enforced by convco via the `commit-msg`/`pre-push` git hooks and the CI PR
check) and do not hand-edit versions for routine changes. `just versions` lists every
crate's current version; `just bump <crate> <level>` is a manual escape hatch.
