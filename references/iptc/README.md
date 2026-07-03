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
  IIM dataset id/name/max-bytes and its XMP path (the `ipmd_top` entries carrying an `IIMid`). It
  contains **no per-property reconciliation rule** — the merge precedence (`ConflictPolicy`,
  XMP-wins default matching exiv2/ExifTool de-facto behaviour) is `gamut-iptc`'s own design. The
  crate's transcribed tables are pinned to this file by
  [`gamut-iptc/tests/techreference.rs`](../../crates/gamut-iptc/tests/techreference.rs); note the
  one place it disagrees with `iim-4.2.pdf` (2:04 max length: 64 text-only octets here vs the
  68-octet wire form there — the crate follows the PDF).

## Conformance

Differential oracle against **exiv2** — a vendored, statically-linked build in
[`tooling/gamut-iptc-oracle`](../../tooling/gamut-iptc-oracle) (from the `third_party/exiv2`
submodule). exiv2's XMP toolkit is disabled in that build (no Expat), so it cross-checks the legacy
IIM dataset stream and the Photoshop IRB; the IPTC-in-XMP property leg is left to the gamut-xmp
oracle. See [`gamut-iptc/STATUS.md`](../../crates/gamut-iptc/STATUS.md).
