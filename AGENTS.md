# gamut

A collection of space-efficient image encoding libraries, organized as a Cargo workspace
under `crates/`.

## Workspace layout

The umbrella crate `gamut` re-exports format crates behind Cargo features; everything builds
on shared primitives. Dependency edges (a crate depends on those to its right):

- **gamut** -- umbrella; optional deps on the format crates, gated by features (`avif`,
  `jxl`, `webp`, `heic`, `vvc`, `av1`, `av2`, `all`). `default = []`.
- **gamut-core** -- `Encoder`/`Decoder` traits, image buffers, `Dimensions`, `Error`. No
  internal deps; everything else depends on it.
- **gamut-color** / **gamut-dsp** / **gamut-bitstream** -- shared primitives. ← core.
- **gamut-isobmff** (AVIF/HEIC container) / **gamut-riff** (WebP container). ← core, bitstream.
- **gamut-av1** / **gamut-av2** / **gamut-jxl** / **gamut-vvc** -- codecs. ← core, color, dsp, bitstream.
- **gamut-avif** ← av1, isobmff, core, color. **gamut-webp** ← +riff. **gamut-heic** ← isobmff, core, color.
- **gamut-cli** (binary named `gamut`) / **gamut-wasm** (cdylib) / **gamut-ffi** (cdylib/staticlib). ← gamut.

### Metadata & dependencies

All cargo metadata is centralized in the root `[workspace.package]` /
`[workspace.dependencies]`. New crates inherit shared fields via `.workspace = true` and set
only their own unique `description`. Add a third-party dependency to the root
`[workspace.dependencies]` first, then reference it with `<dep>.workspace = true`.

Key shared deps:

- **tracing** -- structured diagnostic logging emitted by the encoders; no subscriber is
  configured here (this is a library — the consuming application installs one).
- **thiserror** -- derives `std::error::Error` for the public error enums so callers get
  ergonomic, typed encoding/decoding failures.

## Quality

Validate changes:

```bash
just test            # correctness
just format-check    # formatting
just lint            # lint (Clippy, warnings as errors)
just coverage        # coverage (minimum 80%)
```

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
