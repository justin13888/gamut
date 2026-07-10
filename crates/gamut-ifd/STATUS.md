# gamut-ifd — TIFF/IFD container core implementation status

`gamut-ifd` factors the TIFF Image File Directory structure (`references/tiff/tiff6.pdf` §2, plus
`references/tiff/bigtiff.html` for BigTIFF) out as a shared primitive consumed by both
[`gamut-tiff`](../gamut-tiff) (the TIFF codec, issue #107) and [`gamut-exif`](../gamut-exif) (EXIF
metadata, issue #34). It models *structure only* — byte order, field types, values, the IFD chain,
and the offset-driven read/write spine — never pixels, compression, or photometry.

**Keystone:** the **two-pass offset layout** in the writer ([`write`](src/writer.rs)). Out-of-line
values and following IFDs need absolute offsets that are only known after sizes are fixed, so the
writer plans the layout then back-patches the offset words; a read → write → read round-trip
reproduces the directory exactly. At v1 the symmetry extends to whole sub-IFD trees:
`read_tree(&write(&file)?, tags)? == file`.

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
| P5 | §2 | Sub-IFD pointers + nested directories (the SubIFDs/Exif/GPS offset-tag pattern) | ✅ done (#109 write, #181 read) |
| P6 | §2 | Robustness: offset-loop / overlap / truncation guards + malformed-input corpus | ✅ done (#181) |
| P7 | — | libtiff/exiv2 differential oracle gate (via the consuming codecs — see below) | ✅ via codecs |
| P8 | — | BigTIFF (8-byte offsets/counts, `Long8`/`SLong8`/`Ifd8`) — gated `bigtiff` feature, additive | ✅ done |

P5's **write** side landed with the DNG codec (issue #109): [`write`](src/writer.rs) lays out the
whole IFD *tree* — [`Ifd::set_sub_ifd`](src/entry.rs) attaches children under a pointer tag
(`SubIFDs`/`ExifIFD`/…), the writer places them and synthesises the offset-array field. The **read**
side closed with v1 (issue #181): [`read_tree`](src/reader.rs) follows caller-named pointer tags
(the generic [`read`] cannot know which `LONG` tags are offsets) with depth and cycle guards,
rebuilding the tree `write` flattens; [`read_ifd_at`](src/reader.rs) stays as the per-pointer escape
hatch for lenient decoders (gamut-dng skips unparseable children while hunting the raw IFD,
gamut-exif tolerates malformed sub-IFDs in lenient mode).

P6's corpus ([`tests/robustness.rs`](tests/robustness.rs)) drives `read`/`read_tree` over specific
malformed inputs, a full truncation sweep, an LCG byte-flip fuzz, and an exhaustive single-byte
overwrite sweep, on both variants — and immediately caught (and fixed) a hostile-BigTIFF entry-count
multiply overflow.

P7 is satisfied **through the consuming codecs**, deliberately: `gamut-tiff`'s libtiff oracle
round-trips real TIFF containers byte-for-byte through this crate's reader/writer (including
BigTIFF, multi-IFD, and sub-IFD structure), and `gamut-exif`'s exiv2 oracle parses/round-trips bare
TIFF streams through the same paths (the 0th IFD and the Exif sub-IFD behind the pointer). A direct
oracle dev-dependency here would drag the C-oracle toolchain into this crate's test cycle without
exercising any path the codec gates do not already cover byte-exactly.

## v1 surface (issue #181)

The API was frozen after a full-surface review; the additions and breaks:

- **Spec fix** — multi-string ASCII/UTF-8 values (TIFF 6.0 §2) round-trip: decode strips exactly
  the terminating NUL and preserves interior separators.
- **Fallible `write`** — classic-width overflow (entry count > `u16::MAX`, layout > 4 GiB) is a
  typed error instead of silent truncation; the layout contract (word alignment, structural
  determinism) is documented API.
- **`read_tree`** — write's inverse over sub-IFD trees; sub-IFD groups are tag-sorted (canonical).
- **`Ifd::remove`**, **`Value::as_str`/`as_bytes`/`as_rationals`/`as_srationals`**,
  **`Value::offset_array`**, **`align_word`** — the pieces every consumer had hand-rolled.
- **Single canonical paths** — modules are private; the surface is the crate-root re-export list.
