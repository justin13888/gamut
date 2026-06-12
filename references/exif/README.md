# EXIF (Exchangeable image file format)

Reference specifications for the `gamut-exif` crate (and the shared `gamut-ifd` TIFF/IFD core it
builds on).

## Authoritative editions

- **Exif 3.0 — CIPA DC-008-2024 / JEITA CP-3461** — the current standard. Published freely (English
  translation) by the Camera & Imaging Products Association: <https://www.cipa.jp/std/std-sec_e.html>.
  This is the edition `gamut-exif` targets.
- **Exif 2.32 — CIPA DC-008-2019** — the long-deployed prior edition; retained as the reference for
  legacy tag compatibility (most cameras in the wild emit 2.2–2.32).

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
