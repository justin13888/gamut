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
  `jxl`, `webp`, `heic`, `vvc`, `av1`, `av2`, `tiff`, `dng`, `png`, `jpeg`, `isobmff`, `metadata`, `all`). `default = []`. The `primitives`
  feature additionally re-exports the shared `color`/`dsp`/`bitstream` crates for tooling, the
  `isobmff` feature re-exports the ISOBMFF/HEIF still-image container primitive (the box tree shared
  by avif/heic), the `metadata` feature re-exports the image-metadata primitives, and the `tonemap` feature re-exports
  the tone-mapping primitives; `all` includes all of these.
- **gamut-core** -- `Encoder`/`Decoder` traits, image buffers, `Dimensions`, `Error`. No
  internal deps; everything else depends on it.
- **gamut-color** / **gamut-dsp** / **gamut-bitstream** -- shared primitives. ← core.
- **gamut-tonemap** -- scalar tone-mapping curves (`ToneCurve` + Reinhard/ACES/Hable/Drago
  operators) for HDR→SDR pipelines; sits between `gamut-color`'s transfer functions (linearize)
  and the target SDR re-encode. ← core.
- **gamut-isobmff** (AVIF/HEIC container) / **gamut-riff** (WebP container). ← core, bitstream.
- **gamut-av1** / **gamut-av2** / **gamut-vvc** -- codecs. ← core, color, dsp, bitstream.
- **gamut-jxl** -- JPEG XL codec, uniquely a **wrapper** over the format's reference implementations
  rather than clean-slate (maintainer-approved departure from the pure-Rust rule, issue #243): encode
  wraps **libjxl 0.12.0** (statically, via `gamut-jxl-sys`; target-gated **off wasm32**), decode wraps
  the pure-Rust external `jxl` crate (jxl-rs). ← core, gamut-jxl-sys (encode, non-wasm), external `jxl`.
- **gamut-jxl-sys** -- declarations-only (no fn bodies) `-sys` crate that statically builds and links
  **libjxl 0.12.0** via the BSD-3-Clause `jpegxl-src` (`links = "jxl"`); the native backend for
  gamut-jxl's encoder and its libjxl decode-oracle tests. No gamut deps (C/FFI only); build honors
  `GAMUT_JXL_SYS_SKIP_NATIVE=1` to skip cmake for check-only (cross/MSRV) jobs.
- **gamut-jpeg** -- JPEG-1 (ISO/IEC 10918-1 / ITU-T T.81) codec (issue #28): baseline sequential
  DCT Huffman encoder (gray + YCbCr 4:4:4/4:2:2/4:2:0, JFIF), with the sequential/progressive
  decoder and progressive encoder phased in per its STATUS.md; oracle = libjpeg-turbo (dev-only).
  ← core, color, dsp.
- **gamut-avif** ← av1, isobmff, core, color. **gamut-webp** ← +riff. **gamut-heic** ← isobmff, core, color.
- **gamut-ifd** -- TIFF/IFD container core (byte order, field types, IFD read/write); a low-level
  container primitive (sibling to bitstream), shared by the `gamut-tiff` codec (issue #107) and EXIF
  metadata. ← core. Its optional `bigtiff` feature adds the 64-bit BigTIFF variant. The per-format
  metadata crates (**gamut-exif** ← ifd; **gamut-icc**; **gamut-xmp**; **gamut-iptc** ← xmp) and the
  **gamut-metadata** facade (← exif/xmp/icc/iptc) layer on top, grouped under the umbrella `metadata`
  feature (issue #34); the format crates will consume the facade for embedded metadata.
- **gamut-tiff** -- natively still-image TIFF 6.0; its IFD/tag container is the shared **gamut-ifd**
  primitive (with the `bigtiff` feature), not isobmff/riff. Adds the codec — pixel modes plus its
  compressions (None/PackBits/LZW/CCITT/JPEG). ← ifd, core, color, dsp, bitstream.
- **gamut-dng** -- DNG (Adobe Digital Negative) 1.7.1 raw encoder + decoder (issue #109), a TIFF/EP
  profile on the shared **gamut-ifd** sub-IFD tree (with `bigtiff`). CFA/LinearRaw photometry,
  uncompressed/Deflate/lossless-JPEG, the colour-calibration profile, and EXIF/XMP/ICC metadata.
  Conformance-gated against the headless-built **Adobe DNG SDK** (`tooling/gamut-dng-oracle`).
  ← ifd, bitstream, core. (MSB-first sub-byte sample packing reuses `gamut-bitstream`; `miniz_oxide` for Deflate.)
- **gamut-cli** (binary named `gamut`) / **gamut-wasm** (cdylib) / **gamut-ffi** (cdylib/staticlib). ← gamut.
  `gamut-cli` is the sandbox that exercises the implemented features: it decodes input via the
  third-party `image` crate (PNG/JPEG/PPM) but encodes only with gamut crates, and exposes the
  `primitives` re-exports as inspection subcommands.

## Code correctness

- Correctness: Crate should follow implement the specifications it claims to follow, thoroughly test implementation against oracle. Mutant testing should pass using only non-redundant, high-value tests; exclusions should be done only with strictly strong justification.
- Specification as source of truth: All implementation and tests must be based on the official specifications (in `references/`) and oracle (that is claimed in the crate documentation).
- Design and documentation: Crate exposes public API that is designed to be usable by users without reading documentation, and documented for features not planned or deferred.
- No duplication: Maximally depends on other `gamut` dependencies where appropriate and trusted external code dependencies where approved by maintainers.

## Reference

All codec implementations must follow the official specs that should be attached in `references/`

## Validation

Dev tooling (hk, convco, cargo-llvm-cov, CMake/Ninja/Meson, …) is provisioned by
[mise](https://mise.jdx.dev): run `mise install` and activate mise in your shell. The former
`just` recipes are now mise tasks — `mise tasks` lists them. Validate changes:

```bash
mise run test          # correctness
mise run fmt-check     # formatting (nightly rustfmt for merge-resilient imports; auto-installed)
mise run lint          # lint (Clippy, warnings as errors)
mise run coverage      # coverage (minimum 80%)
mise run mutants       # mutation testing (run `mise install` once; heavier — needs submodules + C toolchain)
mise run check-cross <triple> # cross-compile check (wasm32/aarch64/musl); extended CI, master/manual
mise run check-msrv    # compile on the documented MSRV; extended CI, master/manual
mise run check-commits # commit messages are Conventional Commits
```

The shipped crates are pure Rust — with one deliberate, maintainer-approved exception:
`gamut-jxl`'s `encode` feature statically builds the libjxl reference encoder (cmake + a C++
toolchain at build time; `wasm32` gets a decode-only JXL) via the `gamut-jxl-sys` crate. Beyond
that, the cross-check tests link reference codecs
(libaom, dav1d, libavif, libtiff) built from the `third_party/` git submodules via the dev-only
oracle crates in `tooling/`. libaom — the AV1 reference codec — is the definitive AVIF/AV1
oracle; see [`references/av1`](references/av1/README.md). Running the tests therefore needs the
submodules checked out (`git submodule update --init --recursive`) and the build tools on
`PATH` — CMake/Ninja/Meson come from mise; pkg-config is the one system package
(`apt-get install pkg-config`), and nasm (for the aom/dav1d x86 SIMD) is built from a vendored
source tarball by the oracle build scripts. No system-installed codec binaries are used.

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
commit messages (enforced by convco via the `commit-msg` git hook and the CI PR
check) and do not hand-edit versions for routine changes. `mise run versions` lists every
crate's current version; `mise run bump <crate> <level>` is a manual escape hatch.
