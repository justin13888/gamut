# gamut-webp

`gamut-webp` is a pure-Rust WebP encoder — a VP8/VP8L bitstream wrapped in a RIFF container.

## Goals

Part of the [gamut](../../README.md) workspace, this crate exists to provide WebP **encoding** that
is:

- **Memory-safe on hostile input.** `#![forbid(unsafe_code)]`, deleting the memory-corruption bug
  class behind libwebp's CVE record (e.g. the zero-click, wormable CVE-2023-4863).
- **Clean-slate from the spec.** Implemented directly from the VP8 / VP8L bitstream specs (see
  [`../../references/`](../../references)) rather than wrapping libwebp.
- **Layered on shared crates.** The container comes from [`gamut-riff`](../gamut-riff); the bit-level
  primitives from [`gamut-bitstream`](../gamut-bitstream); color handling from
  [`gamut-color`](../gamut-color).
- **Buildable anywhere `cargo` is.** No C, no nasm — cross-compiles cleanly (wasm32, aarch64, musl).

WebP is one of gamut's three initial focus formats (alongside AVIF and JPEG).

## Usage

No public API yet — implementation pending. It will follow the same shape as
[`gamut-avif`](../gamut-avif): an encoder type implementing [`gamut_core::Encoder`], reachable
through the umbrella crate's `webp` feature.

## Status

Placeholder — implementation pending.

## Roadmap

- Lossless VP8L still-image encoding first (mirrors the AVIF lossless-first M0 approach).
- Lossy VP8 encoding (DCT + quantization).
- Extended WebP: alpha and animation, via [`gamut-riff`](../gamut-riff).

## License

Licensed under either of MIT or Apache-2.0 at your option.
