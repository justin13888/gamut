# ISOBMFF / HEIF (still-image container)

Structural reference for **`gamut-isobmff`** — the ISO Base Media File Format box tree and its HEIF
still-image profile — shared by [`gamut-avif`](../../crates/gamut-avif) and
[`gamut-heic`](../../crates/gamut-heic). It defines the container only (the box tree, item
properties, `iloc` offset layout); the coded bitstream (`av1C`/`hvcC` + samples) belongs to the codec
and its own references (`references/av1`).

## Not vendored (paywalled — cross-checked via oracle)

Both base specifications are ISO-paywalled and cannot be redistributed here:

- **ISO/IEC 14496-12 — ISO Base Media File Format (ISOBMFF).** The box grammar (`Box`/`FullBox`,
  4-byte size + 4-character type), `ftyp`, `meta`, `hdlr`, `pitm`, `iloc`, `iinf`/`infe`,
  `iprp`/`ipco`/`ipma`, and `mdat`.
- **ISO/IEC 23008-12 — HEIF (High Efficiency Image File Format).** The image-item profile of the
  above: `handler_type = "pict"`, the `ispe`/`pixi`/`colr`/`irot`/`imir` item properties, and the
  essential-property rules (also constrained by ISO/IEC 23000-22 **MIAF**).

Conformance is instead verified against:

- **The public AVIF v1.2.0 box table** — vendored at [`references/avif/v1.2.0.html`](../avif),
  which enumerates the exact box set and field layouts a still image uses (§2.2, §6, §8.3, §9.1.1).
- **AV1-ISOBMFF v1.3.0** — vendored at [`references/av1/av1-isobmff/v1.3.0.html`](../av1/av1-isobmff),
  for the `av01` item type and the `av1C` `AV1CodecConfigurationRecord` (§2.3) that AVIF stamps into
  an `ipco` property.
- **A vendored libavif/dav1d differential oracle** (`tooling/libavif-oracle`,
  `third_party/libavif` + `third_party/dav1d`): `gamut-avif/tests/decode_roundtrip.rs` encodes an
  image, has libavif parse the container, and checks the decoded pixels — exercising every box this
  crate writes byte-for-byte. `gamut-isobmff`'s own `read(&write) == img` round-trip and exact-byte
  structure tests pin the layout independently.

## Box set implemented

`ftyp` · `meta` (FullBox v0) · `hdlr` (`pict`) · `pitm` v0 · `iloc` v0 (single extent,
`construction_method` 0) · `iinf` v0 + `infe` v2 · `iprp` = `ipco` + `ipma` v0 · the properties
`ispe`, `pixi`, `colr` (`nclx`), `irot`, `imir`, plus opaque codec-configuration (`av1C`) · `mdat`.
Deferred boxes are listed in [`crates/gamut-isobmff/STATUS.md`](../../crates/gamut-isobmff/STATUS.md).
