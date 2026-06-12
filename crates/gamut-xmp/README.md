# gamut-xmp

`gamut-xmp` is a pure-Rust **XMP (Extensible Metadata Platform)** RDF/XML metadata parser and
serializer.

## Goals

Part of the [gamut](../../README.md) workspace, this crate models the XMP packet embedded in images
(the WebP `XMP ` chunk, the AVIF/HEIF `mime` item, a JPEG `APP1` segment) so the format crates can
read, preserve, and embed XMP. It is:

- **Memory-safe on hostile input.** `#![forbid(unsafe_code)]` — XMP is XML from untrusted files.
- **Clean-slate from the spec.** Implemented from the **Adobe XMP Specification, Parts 1–3**
  (equivalent to ISO 16684; [`../../references/xmp`](../../references/xmp)), modelling the property
  graph (simple / structured / `Bag`·`Seq`·`Alt`, qualifiers, language alternatives) and canonical
  RDF/XML serialization.
- **Dependency-light.** Builds on [`gamut-core`](../gamut-core). XMP uses a constrained RDF/XML
  subset, so it needs no general-purpose XML engine (see the XML-reader open decision in
  [STATUS.md](STATUS.md)).

It is also the substrate for IPTC Photo Metadata Core/Extension, which is serialized *as* XMP —
[`gamut-iptc`](../gamut-iptc) builds on this crate.

## Usage

No public API yet — implementation pending (issue #34). The type declarations sketch the data model
(`XmpMeta`, `XmpProperty`, `XmpValue`, `XmpArray`, `Namespace`/`WellKnownNs`, `XmpPacket`) plus the
`XmpReader` / `XmpWriter` entry points.

## Status

Scaffolding — **under active implementation** (issue #34). See [STATUS.md](STATUS.md).

## License

Licensed under either of MIT or Apache-2.0 at your option.
