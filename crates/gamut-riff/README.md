# gamut-riff

`gamut-riff` provides Resource Interchange File Format (RIFF) container utilities — the chunked
container that WebP is built on.

## Goals

Part of the [gamut](../../README.md) workspace, this crate exists to:

- **Own the WebP container, not the codec.** It will read and write the RIFF chunk structure
  (`RIFF`/`WEBP` plus `VP8 `/`VP8L`/`VP8X`/`ALPH`/`ANIM`… chunks), leaving the VP8/VP8L bitstream to
  [`gamut-webp`](../gamut-webp) — mirroring how [`gamut-isobmff`](../gamut-isobmff) backs AVIF/HEIC.
- **Stay spec-faithful.** Implemented clean-slate from the RIFF and WebP container specs (see
  [`../../references/`](../../references)).
- **Stay memory-safe on hostile input.** `#![forbid(unsafe_code)]`.

## Usage

No public API yet — implementation pending. Once it lands, `gamut-riff` will expose a chunk
reader/writer that [`gamut-webp`](../gamut-webp) drives; most consumers will use it indirectly
through `gamut-webp` rather than directly.

## Status

Placeholder — implementation pending.

## Roadmap

- RIFF chunk writer for the simple-WebP case (`RIFF`/`WEBP` + `VP8 `/`VP8L`).
- Extended WebP (`VP8X`): alpha (`ALPH`), animation (`ANIM`/`ANMF`), and metadata chunks.

## License

Licensed under either of MIT or Apache-2.0 at your option.
