# gamut-iptc — IPTC photo metadata implementation status

Part of the **image metadata primitives** campaign (GitHub issue #34); stabilized to **v1** under
issue #182. Implements IPTC photo metadata (`references/iptc`) in both forms — legacy IIM and IPTC
Core over XMP — building the XMP path on [`gamut-xmp`](../gamut-xmp). Delivered as a stack of
small, individually-reviewable PRs; each PR is independently green
(`mise run test`/`lint`/`fmt-check`/`coverage` ≥ 80%).

**Keystone:** **IIM ↔ XMP reconciliation** — an image may carry the same datum in legacy IIM, in
IPTC-Core XMP, or in both with conflicting values; merging the carriers coherently and writing both
consistently is the genuinely hard part. The IPTC guidelines call for keeping the carriers in sync
but prescribe no single winner, so the precedence knob (`ConflictPolicy`, XMP-wins default matching
exiv2/ExifTool de-facto behaviour) is this crate's own design, applied over the spec's IIM↔XMP
property mapping.

**Oracle:** differential vs **exiv2** (`tooling/gamut-iptc-oracle`, a vendored static build). exiv2's
XMP toolkit is disabled in the oracle build (no Expat), so it cross-checks the legacy IIM dataset
stream and the Photoshop IRB; the IPTC-in-XMP property leg is left to the gamut-xmp oracle. The
schema/tag tables are additionally pinned to the IPTC machine-readable tech reference by
`tests/techreference.rs`.

## Phases

| Phase | Spec | Scope | Status |
| ----- | ---- | ----- | ------ |
| P1 | — | Scaffold: crate, workspace wiring, docs, region-free data-model skeleton | ✅ |
| P2 | Photoshop IRB | Parse + serialize the `8BIM` resource stream; locate the `0x0404` IIM resource | ✅ |
| P3 | IIM 4.2 | IIM dataset stream codec (standard + extended length), 1:90 charset (Latin-1/UTF-8), known-tag table | ✅ |
| P4 | IPTC PMD | IPTC Core over XMP — typed accessors over the `gamut-xmp` property graph (issue #34) | ✅ |
| P5 | IPTC mapping | **Keystone** — IIM ↔ XMP reconciliation (precedence policy + date split/join) | ✅ |
| P6 | — | IIM/IRB writer round-trip + exiv2 differential gate (`tooling/gamut-iptc-oracle`) | ✅ |
| v1 | issue #182 | API finalization (two entry points, published field map, complete Core accessors), strict-write/honest-read error contract, tech-reference drift guard, divan benches, docs | ✅ |

## Deferred / out of scope (v1)

Intentional, documented skips — none lose data on round-trip:

- **IPTC Extension structures** (image regions, artwork/object, licensors, locations shown, …) and
  the structured `Iptc4xmpCore:CreatorContactInfo`: no typed model. They pass through
  `PhotoMetadata::xmp` as raw `gamut-xmp` values untouched.
- **Exotic ISO 2022 character sets**: dataset 1:90 designations other than the spec default
  (decoded as Latin-1, the exiv2/ExifTool de-facto reading of ISO 646 IRV) and UTF-8 (`ESC % G`)
  are reported as `Error::Unsupported`, never mis-decoded.
- **Scalar-shaped repeatable IIM datasets** (2:04 Object Attribute Reference, 2:85 By-line Title):
  repeatable on the wire but mapped to single XMP properties — reconciliation takes the first
  value; the wire side still round-trips every repeat.
- **XMP packet bytes**: parsing/serializing the packet, and the JPEG `APP13`/TIFF tag plumbing,
  belong to `gamut-xmp` and the containers respectively (issue #34). This crate is the semantics
  layer over the in-memory property graph.
- **Length limits are write-side only** (strict-write/honest-read contract): `IptcWriter` rejects
  overlong or unencodable values and IIM-inexpressible `DateCreated`s; the parser accepts and
  preserves overlong wire values rather than reject real-world files.
- **IIM records 3–9**: no named tag-table entries (the table covers the structural record-1 and
  PMD-mapped record-2 datasets); all unmodeled datasets in any record round-trip byte-exact.

## Reference discrepancies

- **2:04 maximum length — 68 vs 64.** The IIM 4.2 PDF defines the wire form as a 3-digit reference
  number, a colon, and up to 64 octets of text (= 68 octets); the PMD tech-reference JSON's
  `IIMmaxbytes` records 64 (the text part only). The crate follows the wire form (68);
  `tests/techreference.rs` pins **both** values so the exception self-invalidates if either source
  changes.
