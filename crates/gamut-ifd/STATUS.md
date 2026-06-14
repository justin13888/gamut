# gamut-ifd — TIFF/IFD container core implementation status

`gamut-ifd` factors the TIFF Image File Directory structure (`references/tiff/tiff6.pdf` §2, plus
`references/tiff/bigtiff.html` for BigTIFF) out as a shared primitive consumed by both
[`gamut-tiff`](../gamut-tiff) (the TIFF codec, issue #107) and [`gamut-exif`](../gamut-exif) (EXIF
metadata, issue #34). It models *structure only* — byte order, field types, values, the IFD chain,
and the offset-driven read/write spine — never pixels, compression, or photometry.

**Keystone:** the **two-pass offset layout** in the writer ([`write`](src/writer.rs)). Out-of-line
values and following IFDs need absolute offsets that are only known after sizes are fixed, so the
writer plans the layout then back-patches the offset words; a read → write → read round-trip
reproduces the directory exactly.

## How it was built

The structural core was migrated from `gamut-tiff`'s self-contained IFD implementation (issue #107):
`gamut-tiff` was developed first with an inlined IFD reader/writer, and the type names here were
authored to mirror it, so the move was near-zero-diff. `gamut-tiff` now consumes this crate (with
the `bigtiff` feature) instead of its own copy; its libtiff differential oracle exercises these exact
read/write code paths byte-for-byte.

## Phases

| Phase | Spec § | Scope | Status |
| ----- | ------ | ----- | ------ |
| P1 | — | Scaffold: crate, workspace wiring, docs, region-free data-model skeleton | ✅ done |
| P2 | §2 | Header + single-IFD reader: II/MM byte order, magic, entry decode for all 12 field types | ✅ done |
| P3 | §2 | Value resolution: inline (≤ offset width) vs out-of-line offsets; multi-IFD chains (`next` links) | ✅ done |
| P4 | §2 | **Keystone** — writer with two-pass offset layout + back-patching; read→write→read round-trip | ✅ done |
| P5 | §2 | Sub-IFD pointers + nested directories (the SubIFDs/Exif/GPS offset-tag pattern) | ◑ writer + `read_ifd_at` (#109) |
| P6 | §2 | Robustness: offset-loop / overlap / truncation guards ✅; fuzz corpus ☐ | ◑ partial |
| P7 | — | libtiff/exiv2 differential oracle gate (gamut-tiff's libtiff oracle covers the shared paths) | ◑ via codec |
| P8 | — | BigTIFF (8-byte offsets/counts, `Long8`/`SLong8`/`Ifd8`) — gated `bigtiff` feature, additive | ✅ done |

P5's **write** side landed with the DNG codec (issue #109): [`write`](src/writer.rs) lays out the
whole IFD *tree* — [`Ifd::set_sub_ifd`](src/entry.rs) attaches children under a pointer tag
(`SubIFDs`/`ExifIFD`/…), the writer places them and synthesises the offset-array field — and
[`read_ifd_at`](src/reader.rs) re-parses a child at a given offset. The generic [`read`] still leaves
a pointer tag as a plain integer value (it cannot know which `LONG` tags are offsets), so automatic
sub-IFD *traversal* on read — plus the P6 fuzz corpus and a dedicated exiv2 oracle — remains for the
EXIF campaign (issue #34), which layers the `Exif\0\0` marker on top of this core.
