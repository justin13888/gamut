# gamut-iptc — IPTC photo metadata implementation status

Part of the **image metadata primitives** campaign (GitHub issue #34). Implements IPTC photo
metadata (`references/iptc`) in both forms — legacy IIM and IPTC Core/Extension over XMP — building
the XMP path on [`gamut-xmp`](../gamut-xmp). Delivered as a stack of small, individually-reviewable
PRs onto the `feat/metadata-primitives` integration branch; each PR is independently green
(`just test`/`lint`/`format-check`/`coverage` ≥ 80%).

**Keystone:** **IIM ↔ XMP reconciliation** — an image may carry the same datum in legacy IIM, in
IPTC-Core XMP, or in both with conflicting values; applying the IPTC mapping guidelines'
precedence/sync rules to merge and to write both consistently is the genuinely hard part.

**Oracle:** differential vs **exiv2** (`tooling/gamut-iptc-oracle`, a vendored static build). exiv2's
XMP toolkit is disabled in the oracle build (no Expat), so it cross-checks the legacy IIM dataset
stream and the Photoshop IRB; the IPTC-in-XMP property leg is left to the gamut-xmp oracle.

## Phases

| Phase | Spec | Scope | Status |
| ----- | ---- | ----- | ------ |
| P1 | — | Scaffold: crate, workspace wiring, docs, region-free data-model skeleton | ✅ |
| P2 | Photoshop IRB | Parse + serialize the `8BIM` resource stream; locate the `0x0404` IIM resource | ✅ |
| P3 | IIM 4.2 | IIM dataset stream codec (standard + extended length), 1:90 charset (Latin-1/UTF-8), known-tag table | ✅ |
| P4 | IPTC PMD | IPTC Core over XMP — typed accessors over the `gamut-xmp` property graph (issue #34) | ✅ |
| P5 | IPTC mapping | **Keystone** — IIM ↔ XMP reconciliation (precedence policy + date split/join) | ✅ |
| P6 | — | IIM/IRB writer round-trip + exiv2 differential gate (`tooling/gamut-iptc-oracle`) | ✅ |
