# EXIF (Exchangeable image file format)

Reference specifications for the `gamut-exif` crate (and the shared `gamut-ifd` TIFF/IFD core it
builds on).

## Authoritative editions (vendored)

- `exif-3.0-dc-008-translation-2023.pdf` — **Exif 3.0 — CIPA DC-008-Translation-2023-E / JEITA
  CP-3461** — the current standard and the edition `gamut-exif` targets. This is the freely-published
  English translation of CIPA DC-008-2023; the later DC-008-2024 is an editorial-corrected reprint of
  the same Exif 3.0. Source: Camera & Imaging Products Association,
  <https://www.cipa.jp/std/std-sec_e.html> (English translation also mirrored at the Internet
  Archive).
- `exif-2.32-dc-008-2019.pdf` — **Exif 2.32 — CIPA DC-008-Translation-2019-E** — the long-deployed
  prior edition; retained as the reference for legacy tag compatibility (most cameras in the wild emit
  2.2–2.32). Source: <https://www.cipa.jp/std/documents/e/DC-X008-Translation-2019-E.pdf>.

Exif's structure **is** a constrained profile of TIFF: an `Exif\0\0` marker followed by a TIFF
stream (byte-order header + IFD chain). The 0th/1st IFDs plus the Exif, GPS, and Interoperability
sub-IFDs are all parsed through the shared [`gamut-ifd`](../../crates/gamut-ifd) primitive, whose
structural reference is **TIFF 6.0** (see [`../tiff`](../tiff)).

## Not vendored (cross-checked via oracle)

- **TIFF/EP — ISO 12234-2** — paywalled. The EP-specific tags are cross-checked against the
  `exiv2`/`libexif` differential oracle rather than vendored, following the same policy as the
  paywalled ISOBMFF/HEIF specs.

## Conformance

Implementations are held to the standard via differential oracles (`exiv2`/`libexif`, C FFI) for
binary round-trip parity, plus **exiftool** as a golden tag-breadth reference (committed JSON
fixtures, regenerated offline). See [`gamut-exif/STATUS.md`](../../crates/gamut-exif/STATUS.md).
