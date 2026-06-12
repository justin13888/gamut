# gamut-jxl

`gamut-jxl` is a pure-Rust JPEG XL (JXL) image encoder.

## Goals

Part of the [gamut](../../README.md) workspace, this crate exists to provide JPEG XL **encoding**
that is:

- **Memory-safe on hostile input.** `#![forbid(unsafe_code)]`.
- **Clean-slate from the spec.** Implemented directly from the JPEG XL specification (see
  [`../../references/`](../../references)) rather than wrapping libjxl.
- **Layered on shared crates.** It will build on [`gamut-color`](../gamut-color),
  [`gamut-dsp`](../gamut-dsp), and [`gamut-bitstream`](../gamut-bitstream) (which will grow the ANS
  entropy coder JXL needs).
- **Permissively licensed**, matching the royalty-free JPEG XL format.

Note: JPEG XL is intentionally **out of scope for now** — the workspace's initial focus is AVIF,
WebP, and JPEG (see the workspace README's "Scope"). This crate is scaffolding and may move or be
dropped; a dedicated effort is likely a better home for a full JXL encoder.

## Usage

No public API yet — implementation pending. It will follow the same shape as
[`gamut-avif`](../gamut-avif): an encoder type implementing [`gamut_core::EncodeImage`], reachable
through the umbrella crate's `jxl` feature.

## Status

Placeholder — implementation pending.

## Roadmap

- Out of scope for the current milestones; revisit once the focus formats (AVIF/WebP/JPEG) mature.

## License

Licensed under either of MIT or Apache-2.0 at your option.
