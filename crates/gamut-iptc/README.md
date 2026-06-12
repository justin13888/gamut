# gamut-iptc

`gamut-iptc` is a pure-Rust **IPTC photo metadata** parser and serializer, covering both the legacy
IIM form and the modern IPTC Photo Metadata (Core + Extension) carried over XMP.

## Goals

Part of the [gamut](../../README.md) workspace, this crate reads, preserves, and embeds IPTC photo
metadata. It is:

- **Memory-safe on hostile input.** `#![forbid(unsafe_code)]`.
- **Clean-slate from the spec.** Implemented from **IPTC-IIM 4.2** and the **IPTC Photo Metadata
  Standard** ([`../../references/iptc`](../../references/iptc)).
- **Two carriers, one model.** Legacy IIM is a binary dataset stream inside a Photoshop Image
  Resource Block (`8BIM`, resource `0x0404`); the modern Core/Extension fields *are* XMP, so that
  path builds on [`gamut-xmp`](../gamut-xmp). The hard part is reconciling the two when both are
  present.

## Usage

No public API yet — implementation pending (issue #34). The type declarations sketch the data model
(`IimDataSet`/`IimRecord`/`IimTag`, `PhotoshopIrb`/`IrbBlock`, `PhotoMetadata`, `IimXmpReconciler`)
plus the `IptcReader` / `IptcWriter` entry points.

## Status

Scaffolding — **under active implementation** (issue #34). See [STATUS.md](STATUS.md).

## License

Licensed under either of MIT or Apache-2.0 at your option.
