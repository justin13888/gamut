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
and WebP-specific helpers: the simple file formats (`write_simple_lossless` / `write_simple_lossy`),
the extended format (`Vp8xHeader`, `write_extended`, `write_extended_with_metadata`), chunk
classification (`WebpChunkId`), and the `ICCP`/`EXIF`/`XMP ` metadata passthrough (`MetadataChunks`).
It is driven by [`gamut-webp`](../gamut-webp); most consumers use it indirectly through that crate
rather than directly.

## Status

The simple-WebP container (RIFF/WEBP header + `VP8 `/`VP8L` chunk read/write) and the extended
format — the `VP8X` feature header plus the `ALPH`, `ICCP`, `EXIF`, and `XMP ` chunks — are
implemented. The remaining chunks (`ANIM`/`ANMF`) are tracked alongside the codec in
[`gamut-webp/STATUS.md`](../gamut-webp/STATUS.md) (section A).

## Roadmap

- RIFF chunk reader/writer for the simple-WebP case (`RIFF`/`WEBP` + `VP8 `/`VP8L`). — done
- Extended WebP (`VP8X`) — alpha (`ALPH`) and metadata chunks (`ICCP`, `EXIF`, `XMP `), read +
  write. — done
- Animation (`ANIM`/`ANMF`) — **out of scope** (recognized FourCCs only). Multi-frame sequences sit
  outside the image-first charter; see
  [`gamut-webp/STATUS.md`](../gamut-webp/STATUS.md#scope-decisions--non-core-feature-paths).

## License

Licensed under either of MIT or Apache-2.0 at your option.
