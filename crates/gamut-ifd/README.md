# gamut-ifd

`gamut-ifd` is a pure-Rust implementation of the **TIFF Image File Directory (IFD) container core**:
the byte-order header, the field-type / value model, the IFD chain, and the offset-driven read/write
spine. It models *structure only* — no pixels, compression, or photometry.

## Goals

Part of the [gamut](../../README.md) workspace, this crate exists because the TIFF/IFD structure is
shared by two otherwise-separate efforts:

- **EXIF** ([`gamut-exif`](../gamut-exif), issue #34) — an EXIF blob is an `Exif\0\0` marker followed
  by a TIFF stream; its 0th/1st IFDs and Exif/GPS/Interop sub-IFDs are exactly IFD chains.
- **TIFF** ([`gamut-tiff`](../gamut-tiff), issue #107) — the TIFF image codec, whose container *is*
  its IFD structure.

Factoring the IFD core out keeps the two from duplicating the fiddly, security-sensitive offset
machinery. It is:

- **Memory-safe on hostile input.** `#![forbid(unsafe_code)]` — IFDs are offset-driven, a classic
  parser-exploit surface (offset loops, truncation, overlapping extents).
- **Endianness-honest.** TIFF carries its own byte order (II/MM); the [`ByteOrder`] is threaded
  through every access rather than fixed at compile time.
- **Dependency-light.** Builds only on [`gamut-core`](../gamut-core).

The public types deliberately mirror `gamut-tiff`'s current structural types so the codec can later
adopt this crate as a near-zero-diff refactor (a tracked, out-of-scope follow-up).

## Usage

No public API yet — implementation pending (issue #34). The type declarations sketch the data model
(`ByteOrder`, `FieldType`, `Value`, `Field`, `Ifd`, `TiffHeader`) the implementation phases flesh
out, plus the `IfdReader` / `IfdWriter` entry points.

## Status

Scaffolding — **under active implementation** (issue #34). See [STATUS.md](STATUS.md).

## License

Licensed under either of MIT or Apache-2.0 at your option.
