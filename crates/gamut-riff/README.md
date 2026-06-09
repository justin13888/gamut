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

`gamut-riff` exposes a RIFF chunk reader (`RiffReader`) and writer (`RiffWriter`), a `FourCc` type,
and WebP-specific helpers (`write_simple_lossless` / `write_simple_lossy`, `WebpChunkId`). It is
driven by [`gamut-webp`](../gamut-webp); most consumers use it indirectly through that crate rather
than directly.

## Status

The simple-WebP container (RIFF/WEBP header + `VP8 `/`VP8L` chunk read/write) is implemented.
Extended-WebP chunks (`VP8X`, `ALPH`, `ANIM`/`ANMF`, metadata) are tracked alongside the codec in
[`gamut-webp/STATUS.md`](../gamut-webp/STATUS.md) (section A).

## Roadmap

- RIFF chunk reader/writer for the simple-WebP case (`RIFF`/`WEBP` + `VP8 `/`VP8L`). — done
- Extended WebP (`VP8X`): alpha (`ALPH`), animation (`ANIM`/`ANMF`), and metadata chunks.

## License

Licensed under either of MIT or Apache-2.0 at your option.
