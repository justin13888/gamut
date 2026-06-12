# gamut-xmp — XMP implementation status

Part of the **image metadata primitives** campaign (GitHub issue #34). Implements XMP
(`references/xmp`, Adobe XMP Parts 1–3 = ISO 16684) as an RDF/XML parser + serializer. Delivered as
a stack of small, individually-reviewable PRs onto the `feat/metadata-primitives` integration branch;
each PR is independently green (`just test`/`lint`/`format-check`/`coverage` ≥ 80%).

**Keystone:** **canonical RDF/XML serialization** (Adobe XMP Part 1 §7). RDF admits many
serializations of the same graph; the canonical form fixes element-vs-attribute encoding, namespace
placement, and array/struct nesting so output is stable, diffable, and round-trippable. Parsing the
(more permissive) input is comparatively routine.

**Open decision — XML reader:** the workspace is near-zero-dependency. XMP uses a *constrained*
RDF/XML subset, so the default is a **hand-rolled** reader; the alternative is adding `quick-xml` to
`[workspace.dependencies]`. To be settled before P2. The `XmpReader`/`XmpWriter` surface is
unaffected either way.

**Oracle:** **exiftool** / Adobe-SDK golden output for tag breadth; **exiv2** for binary round-trip.

## Phases

| Phase | Spec § | Scope | Status |
| ----- | ------ | ----- | ------ |
| P1 | — | Scaffold: crate, workspace wiring, docs, region-free data-model skeleton | ✅ in progress |
| P2 | Part 1 §7 | `xpacket` wrapper + namespace registry + parse simple (literal) properties | ☐ |
| P3 | Part 1 §6 | Structured values, `Bag`/`Seq`/`Alt` arrays, qualifiers, language alternatives | ☐ |
| P4 | Part 2 | Standard schema coverage (dc/xmp/xmpRights/xmpMM/photoshop/exif/tiff) | ☐ |
| P5 | Part 1 §7 | **Keystone** — canonical RDF/XML serialization + packet emit (writable padding) | ☐ |
| P6 | — | exiftool/Adobe golden differential gate | ☐ |
