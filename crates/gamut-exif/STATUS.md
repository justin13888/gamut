# gamut-exif — EXIF implementation status

Part of the **image metadata primitives** campaign (GitHub issue #34). Implements EXIF
(`references/exif`, Exif 3.0 / CIPA DC-008) on top of the shared [`gamut-ifd`](../gamut-ifd)
TIFF/IFD core. Released as **v1** (issue #194).

**Keystone (done):** the **writer round-trip** — re-emitting a valid `Exif\0\0` + TIFF blob through
`gamut-ifd`'s offset-patching writer with the Exif/GPS/Interop sub-IFD pointers, JPEG thumbnail, and
source byte order intact. `parse → to_bytes → parse` is value-level identical in both byte orders.

**Oracle:** differential vs **exiv2** (`tooling/exiv2-oracle`, dev-only FFI over
`Exiv2::ExifParser::decode/encode`) for read/round-trip parity, plus committed **byte-level golden
fixtures** (`tests/fixtures/`, regenerate with `GAMUT_REGEN_GOLDEN=1`).

## Phases

| Phase | Spec § | Scope | Status |
| ----- | ------ | ----- | ------ |
| P1 | — | Scaffold: crate, workspace wiring, docs, region-free data-model skeleton | ✅ |
| P2 | §4.6 | Marker + IFD traversal over `gamut-ifd`: 0th IFD + Exif/GPS/Interop sub-IFD pointers | ✅ |
| P3 | §4.6 | Typed value access (`Rational`/`SRational`/`as_text`) + Exif 3.0 UTF-8 (type 129) | ✅ |
| P4 | §4.6 | Standard CIPA DC-008 tag dictionary (`ExifTag`, macro-table-driven) | ✅ |
| P5 | §4.6.6 | GPS typed model + thumbnail (1st IFD) extraction & JPEG re-embed | ✅ |
| P6 | §4.6 | **Keystone** — writer round-trip (endianness/pointers/thumbnail preserved) | ✅ |
| P7 | §4.6 | MakerNote: opaque passthrough + vendor detection (no per-vendor decode) | ✅ |
| P8 | — | exiv2 differential gate + golden fixtures | ✅ |

## Intentionally deferred (additive under the `#[non_exhaustive]` surface)

- **Per-vendor MakerNote decoding** (Canon/Nikon/Sony/…). The crate preserves the block verbatim
  — and, since issue #263, pins its byte range at the source offset on rewrites so
  vendor-absolute internal offsets stay valid — and detects the vendor from `Make`, but does not
  decode the block (documented on `MakerNote`).
- **exiftool-parity tag breadth** beyond the standard dictionary. Unknown and MakerNote tags still
  round-trip losslessly because the raw `gamut_ifd::Ifd` is retained.
- **Uncompressed strip-based thumbnails** are read (as their directory) but not re-embedded; JPEG
  thumbnails round-trip fully.
- **A reader coverage/validation report** — `gamut-ifd` has the machinery; exposing a toggle on
  `ExifReader` is a non-breaking future addition.
