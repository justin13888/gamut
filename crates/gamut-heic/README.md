# gamut-heic

`gamut-heic` is a pure-Rust HEIC/HEIF encoder — HEVC (H.265) bitstreams wrapped in an ISOBMFF
container.

## Goals

Part of the [gamut](../../README.md) workspace, this crate exists to provide HEIC **encoding** that
is:

- **Memory-safe on hostile input.** `#![forbid(unsafe_code)]`, deleting the memory-corruption bug
  class that has bitten the C HEVC/HEIF stacks.
- **Clean-slate from the spec.** Implemented directly from the HEVC and HEIF specs (see
  [`../../references/`](../../references)) rather than wrapping libde265/libheif.
- **Sharing the AVIF container.** It reuses [`gamut-isobmff`](../gamut-isobmff) — the same ISOBMFF
  box writer that backs AVIF — and color handling from [`gamut-color`](../gamut-color).

Note: HEVC is patent-encumbered, unlike gamut's royalty-free focus formats; this crate is
scaffolding and may move or be dropped as the project's scope sharpens (see the workspace README's
"Scope").

## Usage

No public API yet — implementation pending. It will follow the same shape as
[`gamut-avif`](../gamut-avif): an encoder type implementing [`gamut_core::Encoder`], reachable
through the umbrella crate's `heic` feature.

## Status

Placeholder — implementation pending.

## Roadmap

- HEVC intra still-image encoding, wrapped via [`gamut-isobmff`](../gamut-isobmff) (`hvc1` items).

## License

Licensed under either of MIT or Apache-2.0 at your option.
