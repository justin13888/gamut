# gamut-metadata

`gamut-metadata` is the **unified image-metadata facade** for the gamut workspace. It brings the
per-format crates — [`gamut-exif`](../gamut-exif), [`gamut-xmp`](../gamut-xmp),
[`gamut-icc`](../gamut-icc), and [`gamut-iptc`](../gamut-iptc) — under one `Metadata` model and one
extract/embed surface.

## Goals

- **One model.** A single `Metadata { exif, xmp, icc, iptc }` value an application reads instead of
  juggling four crates.
- **Container-agnostic.** It consumes already-located `MetadataBlock` byte payloads (from the WebP
  `EXIF`/`XMP `/`ICCP` chunks or the AVIF/HEIF `Exif`/`mime`/`colr` items) and produces them back —
  it never parses boxes or chunks itself. That keeps the `format → metadata` dependency thin.
- **The consumer surface.** This is what `gamut-avif`/`gamut-webp`/`gamut-heic` will depend on for
  reading, preserving, and embedding metadata (a later step, out of scope for the scaffold).
- **Cross-format reconciliation.** Harmonising data that appears in more than one standard
  (EXIF ↔ XMP ↔ IPTC), exiftool-style.

## Usage

No public API yet — implementation pending (issue #34). The type declarations sketch the facade
(`Metadata`, `MetadataBlock`, `MetadataExtractor`, `MetadataEmbedder`) and re-export the per-format
crates as `exif` / `xmp` / `icc` / `iptc`.

## Status

Scaffolding — **under active implementation** (issue #34). See [STATUS.md](STATUS.md).

## License

Licensed under either of MIT or Apache-2.0 at your option.
