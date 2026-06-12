# gamut-exif — EXIF implementation status

Part of the **image metadata primitives** campaign (GitHub issue #34). Implements EXIF
(`references/exif`, Exif 3.0 / CIPA DC-008) on top of the shared [`gamut-ifd`](../gamut-ifd)
TIFF/IFD core. Delivered as a stack of small, individually-reviewable PRs onto the
`feat/metadata-primitives` integration branch; each PR is independently green
(`just test`/`lint`/`format-check`/`coverage` ≥ 80%).

**Keystone:** the **writer round-trip** — re-emitting a valid `Exif\0\0` + TIFF blob through
`gamut-ifd`'s offset-patching writer with the Exif/GPS/Interop sub-IFD pointers, thumbnail, and
source byte order intact. The second hard part is **MakerNotes**: vendor-specific blocks with their
own offset/byte-order quirks, and the bulk of the exiftool-parity tag tail.

**Oracle:** differential vs **libexif**/**exiv2** (dev-only FFI) for binary round-trip, plus
**exiftool** golden JSON for tag-breadth coverage (committed fixtures, regenerated offline).

## Phases

| Phase | Spec § | Scope | Status |
| ----- | ------ | ----- | ------ |
| P1 | — | Scaffold: crate, workspace wiring, docs, region-free data-model skeleton | ✅ in progress |
| P2 | §4.6 | Marker + IFD traversal over `gamut-ifd`: 0th IFD + Exif/GPS/Interop sub-IFD pointers; core tag enum | ☐ |
| P3 | §4.6 | Typed value extraction + human-readable formatting (Rational/SRational/Ascii/Undefined) | ☐ |
| P4 | §4.6 | Full standard tag dictionary → exiftool-parity breadth (golden-data driven) | ☐ |
| P5 | §4.6.6 | GPS typed model + thumbnail (1st IFD) extraction | ☐ |
| P6 | §4.6 | **Keystone** — writer round-trip (re-emit valid blob, preserve endianness/pointers) | ☐ |
| P7 | §4.6 | MakerNote framework + first vendors (Canon/Nikon/Sony/…) | ☐ |
| P8 | — | libexif/exiv2 + exiftool differential gate | ☐ |
