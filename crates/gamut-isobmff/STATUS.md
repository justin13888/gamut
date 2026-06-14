# gamut-isobmff — ISOBMFF still-image container implementation status

`gamut-isobmff` factors the ISO Base Media File Format box tree (ISO/IEC 14496-12) and its HEIF
still-image profile (ISO/IEC 23008-12) out as a shared primitive consumed by both
[`gamut-avif`](../gamut-avif) (AV1 still images) and [`gamut-heic`](../gamut-heic) (HEVC still
images). It models *structure only* — the box tree, item properties, and the offset-driven read/write
spine — never the coded bitstream, which is carried opaquely.

**Keystone:** the **single-pass `iloc` back-patch** in the writer ([`write`](src/writer.rs)). Each
item's `extent_offset` is an absolute file offset into `mdat` that is only known after `meta` is
sized, so the writer reserves the slot while emitting `meta` and patches it once `mdat` is placed; a
`read(&write(&img)) == img` round-trip reproduces the model exactly.

## Scope

This crate is **image-first** (the workspace charter forbids inter-frame/sequence coding), so it
covers the HEIF *single-still-image* profile, not the full ISOBMFF movie structure. The authoritative
container ledger is [`gamut-avif/STATUS.md`](../gamut-avif/STATUS.md) §A; this crate implements the
rows marked there as container/`meta` boxes for M0–M5 still images.

## Phases

| Phase | Spec | Scope | Status |
| ----- | ---- | ----- | ------ |
| P1 | 14496-12 §4.2 | Typed box-tree model (`IsoBmffImage`/`Item`/`Property`/`PropertyKind`) + low-level `BoxBuilder`/`BoxReader` | ✅ done |
| P2 | 14496-12; 23008-12 | Writer: `ftyp`, `meta`(`hdlr`/`pitm`/`iloc` v0/`iinf`+`infe` v2/`iprp`), `mdat`; `ispe`/`pixi`/`colr`(nclx)/`irot`/`imir` properties; opaque codec config | ✅ done |
| P3 | 14496-12 | **Keystone** — `iloc` extent back-patch + shared `ipco` dedup → per-item `ipma` | ✅ done |
| P4 | 14496-12; 23008-12 | Reader: bounds-checked box walk, `read(&write)` round-trip, unrecognised property boxes preserved verbatim | ✅ done |
| P5 | — | Robustness: truncation/overrun/size/index guards ✅; counts capped vs remaining ✅; fuzz corpus ☐ | ◑ partial |
| P6 | — | Differential oracle: libavif/dav1d parses the container and reproduces pixels (via `gamut-avif/tests/decode_roundtrip.rs`) | ✅ via codec |

## Deferred / out of scope

Rejected on read (typed `Unsupported`/`InvalidInput`) or preserved opaquely; never written. Each maps
to a milestone in [`gamut-avif/STATUS.md`](../gamut-avif/STATUS.md) §A:

- **M5:** `iloc` v1/v2 + `construction_method` 1 (`idat`)/2 (item), multi-extent items; `idat`;
  `grid`/`dimg`/`iref`/`thmb` derivation; `clap`/`pasp`. (`irot`/`imir` already ship.)
- **M4:** `colr` ICC (`rICC`/`prof`) — preserved as `PropertyKind::Other`, not modelled; Exif/XMP
  items + `cdsc`.
- **M3:** `auxC`/`auxl`/`prem` alpha auxiliary items.
- **M6 / out of charter:** image sequences — `moov`/`trak`/`mdia`/`stbl`, the `av01` sample entry,
  and the `avis` brand. No inter-frame/sequence coding per the workspace charter.

Round-trip is guaranteed for files this crate's `write` produces. Foreign files (multi-extent,
`free` boxes, sequences) are out of scope and are normalised or rejected rather than reproduced.
