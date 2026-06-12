# TIFF 6.0

Structural reference shared by **`gamut-tiff`** (the TIFF codec, issue #107) and **`gamut-ifd`** /
**`gamut-exif`** (the metadata primitives, issue #34): EXIF is a constrained profile of the TIFF
Image File Directory (IFD) / tag structure defined here.

## Authoritative editions

- `tiff6.pdf` — **TIFF Revision 6.0** (Adobe Developers Association, Final — June 3 1992) — the
  baseline structural specification: the byte-order header, IFD layout, the field types, and
  inline-vs-offset value packing. The authoritative reference for the container, tags, and
  baseline/extension features. Freely published by Adobe.
- `bigtiff.html` — the **BigTIFF** extension (libtiff), which keeps TIFF 6.0's structure but widens
  every file offset to 64 bits (magic `43`, a 16-byte header, 20-byte IFD entries, and the
  `LONG8`/`SLONG8`/`IFD8` field types). Canonical source:
  <https://libtiff.gitlab.io/libtiff/specification/bigtiff.html>.

## Not vendored (cross-checked via oracle)

- **TIFF/EP — ISO 12234-2** — paywalled; the EP-specific tags relevant to EXIF are cross-checked via
  the `exiv2`/`libexif`/`libtiff` differential oracles rather than vendored.
