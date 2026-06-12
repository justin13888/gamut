# IPTC photo metadata

Reference specifications for the `gamut-iptc` crate.

## Authoritative editions

- **IPTC-IIM 4.2** (Information Interchange Model) — the legacy binary record/dataset model, still
  embedded in many images (inside a Photoshop Image Resource Block, resource id `0x0404`, within an
  APP13 segment). Published freely by the IPTC: <https://www.iptc.org/std/IIM/4.2/specification/IIMV4.2.pdf>.
- **IPTC Photo Metadata Standard** (latest, e.g. 2023.1) — the modern standard (Core + Extension),
  serialized **as XMP** (so `gamut-iptc` reuses [`gamut-xmp`](../../crates/gamut-xmp) for that path):
  <https://www.iptc.org/standards/photo-metadata/>.
- **IIM ↔ XMP mapping** — the IPTC's guidelines table defining how legacy IIM datasets correspond to
  the XMP Core properties and which wins on conflict (the reconciliation `gamut-iptc` implements).

## Conformance

Differential oracle against **exiv2** (which reads/writes both IIM and IPTC-in-XMP), plus exiftool
golden data; see [`gamut-iptc/STATUS.md`](../../crates/gamut-iptc/STATUS.md).
