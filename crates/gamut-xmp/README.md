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
- **Built on a vetted XML reader.** RDF/XML lexing uses [`quick-xml`](https://docs.rs/quick-xml),
  kept an internal detail — no quick-xml type appears in the public API — atop
  [`gamut-core`](../gamut-core). The serializer is hand-written so the canonical output is pinned.

It is also the substrate for IPTC Photo Metadata Core/Extension, which is serialized *as* XMP —
[`gamut-iptc`](../gamut-iptc) builds on this crate.

## Usage

```rust
use gamut_xmp::{WellKnownNs, XmpMeta};

let dc = WellKnownNs::DublinCore.uri();

let mut meta = XmpMeta::new();
meta.set_lang_alt(dc, "title", "x-default", "My Photo");
meta.set_text(dc, "rights", "(c) 2026");

// Canonical, embeddable packet bytes...
let packet = meta.to_packet();
// ...read straight back into the same graph.
let parsed = XmpMeta::from_packet(&packet).unwrap();
assert_eq!(parsed.get_lang_alt(dc, "title", "x-default"), Some("My Photo"));
```

`XmpMeta::from_packet` accepts a packet with or without the `<?xpacket?>` wrapper (and tolerates a
leading UTF-8 BOM). `XmpMeta::to_packet` / `to_rdf` emit the canonical RDF/XML; `XmpWriter` exposes
the wrapper / writability / padding knobs. `WellKnownNs` supplies the standard schema URIs and
prefixes so you do not hand-write them.

## Status

Implemented: parser + canonical serializer for the full XMP data model (simple / URI / structured /
`Bag`·`Seq`·`Alt`, qualifiers, language alternatives) and the `<?xpacket?>` wrapper, tested against
the spec's canonical examples and round-trip invariants. See [STATUS.md](STATUS.md).

## License

Licensed under either of MIT or Apache-2.0 at your option.
