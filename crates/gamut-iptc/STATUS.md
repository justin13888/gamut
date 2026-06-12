# gamut-iptc — IPTC photo metadata implementation status

Part of the **image metadata primitives** campaign (GitHub issue #34). Implements IPTC photo
metadata (`references/iptc`) in both forms — legacy IIM and IPTC Core/Extension over XMP — building
the XMP path on [`gamut-xmp`](../gamut-xmp). Delivered as a stack of small, individually-reviewable
PRs onto the `feat/metadata-primitives` integration branch; each PR is independently green
(`just test`/`lint`/`format-check`/`coverage` ≥ 80%).

**Keystone:** **IIM ↔ XMP reconciliation** — an image may carry the same datum in legacy IIM, in
IPTC-Core XMP, or in both with conflicting values; applying the IPTC mapping guidelines'
precedence/sync rules to merge and to write both consistently is the genuinely hard part.

**Oracle:** differential vs **exiv2** (which reads/writes both IIM and IPTC-in-XMP), plus exiftool
golden data.

## Phases

| Phase | Spec | Scope | Status |
| ----- | ---- | ----- | ------ |
| P1 | — | Scaffold: crate, workspace wiring, docs, region-free data-model skeleton | ✅ in progress |
| P2 | Photoshop IRB | Parse the `8BIM` resource stream; locate the `0x0404` IIM resource | ☐ |
| P3 | IIM 4.2 | IIM dataset model — Application record (2) descriptive fields | ☐ |
| P4 | IPTC PMD | IPTC Core/Extension over XMP (via `gamut-xmp` typed properties) | ☐ |
| P5 | IPTC mapping | **Keystone** — IIM ↔ XMP reconciliation (precedence + sync rules) | ☐ |
| P6 | — | Writer round-trip for both carriers + exiv2/exiftool differential gate | ☐ |
