# gamut-icc

`gamut-icc` is a pure-Rust **ICC color profile** (ICC.1:2022) parser and serializer.

## Goals

Part of the [gamut](../../README.md) workspace, this crate models the ICC profile blob embedded in
images — the WebP `ICCP` chunk, the AVIF/HEIF `colr` box of type `prof`, a JPEG `APP2` segment — so
the format crates can read, preserve, and embed accurate color characterization. It is:

- **Memory-safe on hostile input.** `#![forbid(unsafe_code)]` — profiles are offset-indexed blobs
  from untrusted files.
- **Clean-slate from the spec.** Implemented from **ICC.1:2022** (profile v4.4, equivalent to
  ISO 15076-1; [`../../references/icc`](../../references/icc)), with v2 read support since most
  embedded profiles are still v2.
- **Dependency-light.** An ICC profile needs neither IFD nor XML machinery, so this crate builds
  only on [`gamut-core`](../gamut-core) — distinct from CICP color signaling, which lives in
  [`gamut-color`](../gamut-color).

## Usage

No public API yet — implementation pending (issue #34). The type declarations sketch the data model
(`ProfileHeader`, `DeviceClass`, `ColorSpace`, `RenderingIntent`, `TagSignature`/`TagEntry`,
`TagType`, `IccProfile`) plus the `IccReader` / `IccWriter` entry points.

## Status

Scaffolding — **under active implementation** (issue #34). See [STATUS.md](STATUS.md).

## License

Licensed under either of MIT or Apache-2.0 at your option.
