# gamut-exif

`gamut-exif` is a pure-Rust **EXIF** (Exif 3.0 / CIPA DC-008) image-metadata parser and serializer.

## Goals

Part of the [gamut](../../README.md) workspace, this crate models the EXIF blob embedded in images
(the JPEG `APP1` payload, the WebP `EXIF` chunk, the PNG `eXIf` chunk, the AVIF/HEIF `Exif` item) so
the format crates can read, preserve, and embed camera/capture metadata. It is:

- **Memory-safe on hostile input.** `#![forbid(unsafe_code)]` — EXIF is offset-driven TIFF from
  untrusted files.
- **Spec-faithful.** Implemented from **Exif 3.0** (CIPA DC-008;
  [`../../references/exif`](../../references/exif)), with 2.32 legacy tag compatibility, and
  differentially tested against **exiv2** plus byte-level golden fixtures.
- **Layered on the shared IFD core.** EXIF *is* a constrained TIFF stream, so the IFD structure,
  byte order, value model, and offset machinery come from [`gamut-ifd`](../gamut-ifd) (whose
  [`Value`] is re-exported here rather than duplicated); this crate adds the EXIF tag dictionary, the
  typed GPS/thumbnail projections, the Exif/GPS/Interop sub-IFD layout, and MakerNote handling.

## Usage

[`Exif::parse`] reads a blob (with or without the `Exif\0\0` marker) and [`Exif::to_bytes`]
re-serialises it, preserving the source byte order. Read tags through the typed accessors or the
[`ExifTag`] catalogue; reach the raw directories as [`gamut_ifd::Ifd`]s when you need them.

```rust
use gamut_exif::{Exif, ExifTag, Value};

# fn demo(bytes: &[u8]) -> Result<(), gamut_exif::ExifError> {
let exif = Exif::parse(bytes)?;
println!("{:?} {:?}", exif.make(), exif.model());
if let Some(gps) = exif.gps() {
    println!("{:?}, {:?}", gps.latitude_deg(), gps.longitude_deg());
}
let jpeg = exif.thumbnail_bytes();          // the embedded JPEG thumbnail, if any

let mut edited = exif;
edited.set_tag(ExifTag::Software, Value::Ascii("gamut".into()));
let out = edited.to_bytes();                // Exif\0\0 + TIFF, ready to re-embed
# let _ = (jpeg, out);
# Ok(())
# }
```

For a bare TIFF stream (PNG `eXIf` / WebP `EXIF`) or a byte-order override, use [`ExifWriter`];
[`ExifReader`] carries the read-side options (`require_marker`, `strict`).

## Scope

v1 covers the **standard CIPA DC-008 tag dictionary** ([`ExifTag`]), full read/write round-trips
over `gamut-ifd`, the typed [`GpsInfo`] projection, and JPEG thumbnails. Intentionally deferred (and
designed to be added without breaking the 1.0 API — the catalogue and vendor enums are
`#[non_exhaustive]`):

- **Per-vendor MakerNote decoding.** The `MakerNote` block is preserved verbatim and its vendor
  detected ([`MakerNoteVendor`]), but not decoded. Re-serialising relocates the block, so a vendor's
  TIFF-absolute internal offsets can go stale — round-trip is guaranteed at the *value* level, not
  byte-for-byte; keep the original blob if you need offset-correct MakerNotes.
- **exiftool-parity tag breadth** beyond the standard dictionary (unknown tags still round-trip
  losslessly via the raw `Ifd`).
- **Uncompressed strip-based thumbnails** are read but not re-embedded (JPEG thumbnails are).

## Status

Implemented and released as **v1** (issue #194). See [STATUS.md](STATUS.md).

## License

Licensed under either of MIT or Apache-2.0 at your option.
