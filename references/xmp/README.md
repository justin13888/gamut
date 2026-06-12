# XMP (Extensible Metadata Platform)

Reference specifications for the `gamut-xmp` crate.

## Authoritative editions

- **ISO 16684-1:2019** (XMP Part 1: Data model, serialization and core properties) and
  **ISO 16684-2** (Part 2: Description of standard schemas) — the formal standard. ISO editions are
  paywalled.
- **Adobe XMP Specification, Parts 1–3** (2012/2016) — freely published by Adobe and technically
  equivalent to the ISO text; this is the working reference used for implementation:
  <https://developer.adobe.com/xmp/docs/XMPSpecifications/>.
  - Part 1 — Data model, serialization (RDF/XML), and the `xpacket` wrapper.
  - Part 2 — Standard schemas (dc, xmp, xmpRights, xmpMM, photoshop, exif-in-xmp, tiff-in-xmp, …).
  - Part 3 — Storage in files (how XMP packets are embedded per container).

XMP is RDF/XML wrapped in an `<?xpacket?>` processing instruction. `gamut-xmp` models the property
graph (simple / structured / `Bag`·`Seq`·`Alt` arrays, qualifiers, language alternatives) and the
canonical RDF/XML serialization.

> **Open decision (XML reader):** the workspace is near-zero-dependency. Whether the constrained
> RDF/XML subset XMP uses is parsed by a hand-rolled reader (default) or a vetted crate
> (`quick-xml`) is recorded in [`gamut-xmp/STATUS.md`](../../crates/gamut-xmp/STATUS.md) and decided
> before the P2 implementation phase.

## Conformance

**exiftool** / Adobe-SDK golden output as the breadth reference; `exiv2` for binary round-trip.
