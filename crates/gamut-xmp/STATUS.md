# gamut-xmp — XMP implementation status

Part of the **image metadata primitives** campaign (GitHub issue #34). Implements XMP
(`references/xmp`, Adobe XMP Parts 1–3 = ISO 16684) as an RDF/XML parser + canonical serializer.

**Keystone:** **canonical RDF/XML serialization** (Adobe XMP Part 1 §7). RDF admits many
serializations of the same graph; the canonical form fixes element-vs-attribute encoding, namespace
placement, and array/struct nesting so output is stable, diffable, and round-trippable. Parsing the
(more permissive) input is comparatively routine.

## Settled decisions

- **XML reader — `quick-xml`.** RDF/XML lexing uses `quick-xml` (a `[workspace.dependencies]` entry),
  kept an internal detail: no quick-xml type appears in the public API (parse failures surface as
  `XmpError`), so the backend can change without a breaking change. The serializer is hand-written to
  pin the canonical byte form.
- **Encoding — UTF-8, no BOM.** Packets are read and written as UTF-8; a leading BOM is tolerated on
  read but never emitted. Part 1 §7.1 also allows UTF-16/32, reported as `XmpError::Encoding`.
- **Conformance oracle — exiv2.** exiv2 bundles Adobe's XMPCore, so it backs both the "Adobe-SDK"
  and "exiv2" checks; the differential gate lands in a follow-up PR (P6). Byte-exact correctness is
  pinned independently by golden vectors transcribed from the Part 1 examples (`tests/golden.rs`).

## Phases

| Phase | Spec § | Scope | Status |
| ----- | ------ | ----- | ------ |
| P1 | — | Scaffold: crate, workspace wiring, docs, region-free data-model skeleton | ✅ done |
| P2 | Part 1 §7 | `xpacket` wrapper + namespace registry + parse simple (literal) properties | ✅ done |
| P3 | Part 1 §6 | Structured values, `Bag`/`Seq`/`Alt` arrays, qualifiers, language alternatives | ✅ done |
| P4 | Part 2 | Standard schema coverage (dc/xmp/xmpRights/xmpMM/photoshop/exif/tiff/…) | ✅ done |
| P5 | Part 1 §7 | **Keystone** — canonical RDF/XML serialization + packet emit (writable padding) | ✅ done |
| P6 | — | exiv2 differential conformance gate | ☐ follow-up PR |

## Known limitations

- A default `xml:lang` declared on an `rdf:Description` (Part 1 §7.8) is not propagated to the
  properties it scopes; per-property/per-item `xml:lang` is fully supported.
- Only UTF-8 packets are read/written (see the encoding decision above).
