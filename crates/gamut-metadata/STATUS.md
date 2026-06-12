# gamut-metadata — facade implementation status

Part of the **image metadata primitives** campaign (GitHub issue #34). The facade that unifies
[`gamut-exif`](../gamut-exif), [`gamut-xmp`](../gamut-xmp), [`gamut-icc`](../gamut-icc), and
[`gamut-iptc`](../gamut-iptc). Delivered as small, individually-reviewable PRs onto the
`feat/metadata-primitives` integration branch; each PR is independently green
(`just test`/`lint`/`format-check`/`coverage` ≥ 80%).

**Keystone:** **lossless cross-carrier round-trip** — extract → embed must reproduce the metadata a
container can re-write faithfully, including reconciling data that appears in more than one standard.

**Consumer integration (out of scope for this campaign):** the format crates
(`gamut-avif`/`gamut-webp`/`gamut-heic`) gaining a `gamut-metadata` dependency to read/preserve/embed
metadata (their respective STATUS M4 milestones). The dependency direction — `format → gamut-metadata
→ per-format crates` — is settled here; the wiring is a later step.

## Phases

| Phase | Scope | Status |
| ----- | ----- | ------ |
| P1 | Scaffold: crate, workspace wiring, docs, region-free facade skeleton (`Metadata`, `MetadataBlock`, extractor/embedder) | ✅ in progress |
| P2 | Extract: dispatch each `MetadataBlock` to its per-format parser into `Metadata` | ☐ |
| P3 | Embed: serialize `Metadata` back to per-format byte blocks | ☐ |
| P4 | Cross-format reconciliation (EXIF ↔ XMP ↔ IPTC harmonisation) | ☐ |
| P5 | Format-crate integration: avif/webp/heic consume the facade (records the planned edge; itself a later campaign) | ⊘ out of scope |
