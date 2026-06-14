# IPTC photo metadata

Reference specifications for the `gamut-iptc` crate.

## Authoritative editions (vendored)

- `iim-4.2.pdf` — **IPTC-IIM 4.2** (Information Interchange Model) — the legacy binary record/dataset
  model, still embedded in many images (inside a Photoshop Image Resource Block, resource id `0x0404`,
  within an APP13 segment). Published freely by the IPTC:
  <https://www.iptc.org/std/IIM/4.2/specification/IIMV4.2.pdf>.
- `iptc-photo-metadata-2025.1.html` — **IPTC Photo Metadata Standard 2025.1** — the modern standard
  (Core + Extension), serialized **as XMP** (so `gamut-iptc` reuses
  [`gamut-xmp`](../../crates/gamut-xmp) for that path). Vendored snapshot of the canonical
  specification page: <https://www.iptc.org/std/photometadata/specification/IPTC-PhotoMetadata>.
- `iptc-pmd-techreference_2025.1.json` — the IPTC's **machine-readable technical reference** for the
  PMD standard, which doubles as the authoritative **IIM ↔ XMP mapping**: each property records its
  IIM dataset, XMP path, and reconciliation rule (the merge `gamut-iptc` implements).

## Conformance

Differential oracle against **exiv2** — a vendored, statically-linked build in
[`tooling/gamut-iptc-oracle`](../../tooling/gamut-iptc-oracle) (from the `third_party/exiv2`
submodule). exiv2's XMP toolkit is disabled in that build (no Expat), so it cross-checks the legacy
IIM dataset stream and the Photoshop IRB; the IPTC-in-XMP property leg is left to the gamut-xmp
oracle. See [`gamut-iptc/STATUS.md`](../../crates/gamut-iptc/STATUS.md).
