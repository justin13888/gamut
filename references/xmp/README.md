# XMP (Extensible Metadata Platform)

Reference specifications for the `gamut-xmp` crate.

## Authoritative editions (vendored)

- **Adobe XMP Specification, Parts 1–3** — freely published by Adobe, technically equivalent to the
  ISO text, and the working reference `gamut-xmp` implements against. Adobe's own download host
  (`wwwimages2.adobe.com`) has since gone offline; the canonical PDFs are vendored here:
  - `xmp-part1.pdf` — Part 1: Data model, serialization (RDF/XML), and the `xpacket` wrapper (Apr 2012).
  - `xmp-part2.pdf` — Part 2: Standard schemas (dc, xmp, xmpRights, xmpMM, photoshop, exif-in-xmp,
    tiff-in-xmp, …) (Aug 2016).
  - `xmp-part3.pdf` — Part 3: Storage in files (how XMP packets are embedded per container) (Jan 2020).

  Canonical index (links now dead but documents unchanged):
  <https://github.com/adobe/xmp-docs/blob/master/Specifications.md>.

## Not vendored (paywalled — Adobe equivalents shipped)

- **ISO 16684-1:2012/2019** (Part 1: Data model, serialization and core properties) and
  **ISO 16684-2:2014** (Part 2: Description of XMP schemas using RELAX NG) — the formal ISO standard,
  paywalled. The Adobe Parts 1–3 above are the technically-equivalent free text used in their place.

XMP is RDF/XML wrapped in an `<?xpacket?>` processing instruction. `gamut-xmp` models the property
graph (simple / structured / `Bag`·`Seq`·`Alt` arrays, qualifiers, language alternatives) and the
canonical RDF/XML serialization.

> **Open decision (XML reader):** the workspace is near-zero-dependency. Whether the constrained
> RDF/XML subset XMP uses is parsed by a hand-rolled reader (default) or a vetted crate
> (`quick-xml`) is recorded in [`gamut-xmp/STATUS.md`](../../crates/gamut-xmp/STATUS.md) and decided
> before the P2 implementation phase.

## Conformance

**exiftool** / Adobe-SDK golden output as the breadth reference; `exiv2` for binary round-trip.
