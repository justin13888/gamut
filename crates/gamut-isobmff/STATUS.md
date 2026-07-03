# gamut-isobmff — ISOBMFF still-image container implementation status

`gamut-isobmff` factors the ISO Base Media File Format box tree (ISO/IEC 14496-12) and its HEIF
still-image profile (ISO/IEC 23008-12) out as a shared primitive consumed by both
[`gamut-avif`](../gamut-avif) (AV1 still images) and [`gamut-heic`](../gamut-heic) (HEVC still
images). It models *structure only* — the box tree, item properties, item references, and the
offset-driven read/write spine — never the coded bitstream, which is carried opaquely.

**Keystone:** the **single-pass `iloc` back-patch** in the writer ([`write`](src/writer.rs)). Each
item's `extent_offset` is an absolute file offset into `mdat` that is only known after `meta` is
sized, so the writer reserves the slot while emitting `meta` and patches it once `mdat` is placed; a
`read(&write(&img)?) == img` round-trip reproduces the model exactly.

## Scope

This crate is **image-first** (the workspace charter forbids inter-frame/sequence coding), so it
covers the HEIF *still-image* profile, not the full ISOBMFF movie structure. The authoritative
container ledger is [`gamut-avif/STATUS.md`](../gamut-avif/STATUS.md) §A; the v1 surface covers the
container rows of every planned milestone (M0–M5), so future codec work is additive here — new
`Item`/`Property` values, never a model reshape.

**Asymmetric by design:** the writer *normalises* (always the smallest box versions — `pitm` v0,
`iloc` v0 single-extent into `mdat`, `infe` v2, `iref` v0, 16-bit item ids — validated up front by
the fallible [`write`]); the reader additionally accepts the foreign-encoder repertoire (`iloc`
v1/v2, `idat` placement, multi-extent payloads, 32-bit item ids, 16-bit `ipma` indices). Round-trip
is therefore model-exact for files this crate writes, and *equivalent-but-normalised* for foreign
files.

## Phases

| Phase | Spec | Scope | Status |
| ----- | ---- | ----- | ------ |
| P1 | 14496-12 §4.2 | Typed box-tree model (`IsoBmffImage`/`Item`/`ItemReference`/`EntityGroup`/`Property`/`PropertyKind`) + low-level `BoxBuilder`/`BoxReader` | ✅ done |
| P2 | 14496-12; 23008-12 | Writer: `ftyp`, `meta` (`hdlr`/`pitm`/`iloc` v0/`iinf`+`infe` v2/`iref` v0/`iprp`/`grpl`), `mdat`; `ispe`/`pixi`/`colr` (`nclx`+ICC)/`irot`/`imir`/`clap`/`pasp`/`auxC`/`clli` properties; opaque codec config; model validation (typed errors, no silent truncation) | ✅ done |
| P3 | 14496-12 | **Keystone** — `iloc` extent back-patch + shared `ipco` dedup → per-item `ipma` (8/16-bit forms) | ✅ done |
| P4 | 14496-12; 23008-12 | Reader: bounds-checked box walk; foreign-file repertoire (`iloc` v0–v2, `mdat`/`idat`, base offsets, 0/4/8-byte fields, multi-extent, `pitm`/`ipma` v0–v1, `infe` v2–v3, `iref` v0–v1); `read(&write)` round-trip; unrecognised property boxes preserved verbatim | ✅ done |
| P5 | — | Robustness: truncation/overrun/size/index guards ✅; counts never trusted for allocation ✅; total payload capped at input size (anti amplification) ✅; spec fixtures independent of the writer ✅; fuzz corpus ☐ | ◑ partial |
| P6 | — | Differential oracle: libavif/dav1d parses the container and reproduces pixels (via `gamut-avif/tests/decode_roundtrip.rs`) | ✅ via codec |

## Payload helpers

- **`ImageGrid`** (23008-12 §6.6.2.3.2) — `ImageGrid::parse`/`to_bytes` types the `grid`
  derived-image payload (tile rows/columns + assembled output size). A `grid` item's `dimg`
  references and payload bytes already round-trip through the box model; this helper types the
  payload geometry for consumers, while the tile payloads stay opaque. Additive (semver-minor).

## Deferred (planned, additive — no model reshape)

- **Typed `mdcv`/`cclv`/`amve`/`reve`/`ndwt` HDR properties.** Carried verbatim as
  `PropertyKind::Other` (exactly what libavif does — it types only `clli`, which we also type).
  No vendored primary source pins their field layouts yet; `PropertyKind` is `#[non_exhaustive]`
  so typing one later is semver-minor.
- **Writer emission options** (`idat` placement, multi-extent, `iloc` v1/v2, 32-bit-id boxes).
  The writer intentionally normalises; if a consumer ever needs layout control it arrives as a
  separate options-taking entry point (semver-minor), not a change to `write`.
- **Fuzz corpus** (P5). The reader is bounds-checked, allocation-capped, and fixture-tested; a
  libFuzzer/AFL corpus remains the missing hardening step.

## Likely out of scope (rejected with a typed error; revisited only if the charter changes)

- **Image sequences/tracks** — `moov`/`trak`/`mdia`/`stbl`, the `av01`/`hvc1` sample entries, the
  `avis` brand (gamut-avif M6). The workspace charter forbids inter-frame/sequence coding;
  `read` rejects `moov`/`trak` as `Unsupported`.
- **Item protection** — `ipro`/`sinf`, `infe` `item_protection_index != 0` (DRM machinery).
- **External data references** — `dinf`/`dref`, `iloc` `data_reference_index != 0` (payloads in
  other files; a still image is self-contained).
- **`iloc` `construction_method` 2** (offsets into another item's payload) — unused by any known
  still-image encoder.
- **`uri ` items** (23008-12 URI-typed metadata) — unused by AVIF/HEIC; `Exif` and `mime`
  (XMP/MPEG-7) items cover real metadata.
- **64-bit box sizes (`largesize`), `size == 0` boxes, files ≥ 4 GiB** — a still image never
  approaches them; every offset/length this crate writes is 32-bit.
- **Non-`pict` handlers** — a `meta` that is not a HEIF image (e.g. ID3 metadata containers).

Round-trip is guaranteed for files this crate's `write` produces (`read(&write(&img)?) == img`,
checked by validation rather than assumed). Foreign files read into an equivalent normalised model;
what they lose is placement/version detail (`idat` vs `mdat`, extent splits, box versions), never
item data, properties, references, or groups.
