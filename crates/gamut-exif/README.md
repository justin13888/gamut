# gamut-exif

`gamut-exif` is a pure-Rust **EXIF** (Exif 3.0 / CIPA DC-008) image-metadata parser and serializer.

## Goals

Part of the [gamut](../../README.md) workspace, this crate models the EXIF blob embedded in images
(the JPEG `APP1` payload, the WebP `EXIF` chunk, the AVIF/HEIF `Exif` item) so the format crates can
read, preserve, and embed camera/capture metadata. It is:

- **Memory-safe on hostile input.** `#![forbid(unsafe_code)]` — EXIF is offset-driven TIFF from
  untrusted files.
- **Clean-slate from the spec.** Implemented from **Exif 3.0** (CIPA DC-008-2024;
  [`../../references/exif`](../../references/exif)), with 2.32 legacy compatibility.
- **Layered on the shared IFD core.** EXIF *is* a constrained TIFF stream, so the IFD structure,
  byte order, and offset machinery come from [`gamut-ifd`](../gamut-ifd); this crate adds the EXIF
  tag dictionary, value interpretation, the Exif/GPS/Interop sub-IFD layout, the thumbnail, and the
  vendor MakerNote dialects.

The long-term goal (issue #34) is **exiftool-class tag coverage**.

## Usage

No public API yet — implementation pending (issue #34). The type declarations sketch the data model
(`Exif`, `ExifTag`/`IfdKind`, `ExifValue`, `GpsInfo`/`GpsCoordinate`, `MakerNote`/`MakerNoteVendor`)
plus the `ExifReader` / `ExifWriter` entry points.

## Status

Scaffolding — **under active implementation** (issue #34). See [STATUS.md](STATUS.md).

## License

Licensed under either of MIT or Apache-2.0 at your option.
