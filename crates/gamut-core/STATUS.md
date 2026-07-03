# gamut-core — foundational primitives status

**v1 stabilization: GitHub issue #177.** `gamut-core` is the format-neutral foundation every gamut
codec builds on: the workspace-wide `Error`/`Result`, the `EncodeImage`/`DecodeImage` traits, the
branded `ImageRef`/`ImageBuf` interleaved buffers, the `Dimensions` value type, the sealed
`Pixel`/`Sample`/`ColorModel` pixel vocabulary, and the shared `luminance` reference constants. It
has **no dependency on the format crates** — everything else depends on it, never the other way
around — and is `#![forbid(unsafe_code)]`.

**Keystone:** the **branded, length-validated buffer**. `len == width * height * P::CHANNELS` (with
non-empty, non-overflowing dimensions) is checked exactly once, at `ImageRef::new` / `ImageBuf::new`,
and the pixel brand `P` makes a layout mismatch (CMYK bytes into an RGBA encoder) a *compile* error
rather than a silent reinterpretation. A codec then pulls the raw slice back out with
`ImageRef::as_samples` at zero cost, so its hot loop is byte-identical to operating on a bare `&[u8]`
while never re-checking the invariant.

## Public surface (frozen at v1)

| Item | Shape | Openness |
| ---- | ----- | -------- |
| `Error` / `Result` | 2 variants (`InvalidInput`/`Unsupported`), each `&'static str` | `#[non_exhaustive]` — new variants are additive |
| `Dimensions` | plain `{ width, height }` value type + `new`/`num_pixels`/`sample_count`/`is_empty` | public fields; length invariant lives on the buffers |
| `EncodeImage<P>` / `DecodeImage<P>` | one `impl` per supported layout `P`; object-safe | sealed via the `Pixel` bound |
| `ImageRef<'a, P>` / `ImageBuf<P>` | borrowed / owned interleaved buffers | length-validated at construction |
| `Pixel` / `Sample` / `ColorModel` | compile-time layout descriptors | `Pixel`/`Sample` **sealed**; `ColorModel` `#[non_exhaustive]` |
| 11 pixel markers | `Gray8` `Bilevel` `Indexed8` `Rgb8` `Rgba8` `Cmyk8` `GrayAlpha8` `Gray16` `Rgb16` `Rgba16` `GrayAlpha16` | closed set, defined only here |
| `luminance::*` | `SDR_REFERENCE_WHITE_NITS` / `HDR_REFERENCE_WHITE_NITS` / `PQ_PEAK_NITS` | shared by `gamut-color` + `gamut-tonemap` |

Adding items (new `Error`/`ColorModel` variants, more buffer helpers, more markers) stays
backward-compatible; removing or reshaping any of the above would not.

## Settled design decisions (intentional, not gaps)

- **Interleaved `u8` / `u16` only.** `Sample` is sealed over `u8`/`u16`. Planar layouts and a stream's
  *coded* bit depth are codec concerns and live in [`gamut-color`](../gamut-color) (`Planar8`,
  `BitDepth`); raw/mosaic (CFA) imagery lives in [`gamut-dng`](../gamut-dng) (`RawImage`). Core stays
  the interleaved-still-image vocabulary the 8/16-bit codecs (png, tiff, avif, webp) share.
- **Open where growth is additive, sealed where it must not be.** `Error` and `ColorModel` are
  `#[non_exhaustive]`; `Pixel`/`Sample` are sealed so the set of pixel layouts is closed and defined
  only here.
- **The length invariant lives on the buffers, not on `Dimensions`.** `Dimensions` keeps public
  fields for ergonomic literals; non-emptiness and the length product are enforced once, at buffer
  construction, so codecs receive a known-good buffer and never re-check.
- **Static error messages.** `Error` payloads are `&'static str`. This is *the* workspace error type
  — every crate returns `gamut_core::Result` and none defines its own — so the two variants are
  deliberately minimal; richer dynamic context is deferred (see below), addable without a break.
- **`luminance` lives here, not in `gamut-color`.** Both `gamut-color` and `gamut-tonemap` need the
  reference nit levels, and `gamut-tonemap` depends only on core at runtime; putting the constants
  here keeps a single authoritative definition without coupling tonemap to color.
- **Typed errors, no panics in library paths.** No `unwrap`/`expect`/`panic!` outside tests; the
  documented `ImageRef::row`/`pixel` panics are the idiomatic out-of-bounds-index behaviour (like
  `slice[i]`), not error handling.

## Deferred / tracked follow-ups (all additive — none blocks v1)

- **Dynamic error context.** `Error::InvalidInput`/`Unsupported` carry only `&'static str`, so a
  boundary that funnels a richer foreign error into core drops the dynamic detail (e.g. `gamut-xmp`'s
  7-variant error collapses to two static strings; `gamut-dng` discards the Deflate source). Because
  `Error` is `#[non_exhaustive]`, a future variant carrying an owned message and/or a boxed source
  can land without a breaking change; deferred until a caller needs it.
- **A shared palette primitive.** `Indexed8` is a first-class buffer brand but has no `EncodeImage`
  path, because the single-buffer trait shape cannot carry a palette table; `gamut-png` and
  `gamut-tiff` each define their own palette type today. A shared palette primitive (alongside
  `Indexed8`) is new, additive surface if the duplication proves worth removing.
- **`decode_image_into` buffer reuse.** The provided `DecodeImage::decode_image_into` override point
  (reuse `dst`'s allocation via `ImageBuf::as_mut_samples`) is documented but not yet used by any
  codec; it stays as an affordance for callers that decode many same-sized frames.

## Validation

Backed by inline unit tests (each type's own contract) plus `tests/surface.rs`, which drives the
crate through its **public API only** (a toy `EncodeImage` → `DecodeImage` round-trip) so the surface
is proven self-sufficient. The crate-root doctest keeps the documented usage example compiler-checked.
No benches: core has no computational hot path — its buffers are zero-cost branding and validation is
a single length check — so there is nothing meaningful to measure (the workspace bench harness scopes
to real-surface crates). Gates: `mise run test` / `lint` (`clippy -D warnings`, `missing_docs` fatal)
/ `fmt-check` / `coverage` (≥ 80%).
