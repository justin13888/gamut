This directory holds the specifications the `gamut-tiff` crate is implemented against.

- `tiff6.pdf` — the TIFF 6.0 base specification (Adobe Developers Association, Final, June 3
  1992). The authoritative reference for the container, tags, and baseline/extension features.
- `bigtiff.html` — the BigTIFF extension (libtiff), which keeps TIFF 6.0's structure but widens
  every file offset to 64 bits (magic `43`, a 16-byte header, 20-byte IFD entries, and the
  `LONG8`/`SLONG8`/`IFD8` field types). Canonical source:
  <https://libtiff.gitlab.io/libtiff/specification/bigtiff.html>.
