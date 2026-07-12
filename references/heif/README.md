# HEIF (High Efficiency Image File Format) — decode / container-parsing reference

Reference for **[`gamut-heic`](../../crates/gamut-heic)** — the HEIF still-image container and its
HEVC-intra image items. gamut is **decode-only** for this format: these crates parse and decode
HEIF/HEIC, they **do not encode it**. HEIF is an image-item profile of the ISO Base Media File
Format, so the box tree itself (`ftyp`/`meta`/`hdlr`/`iloc`/`iinf`/`iprp`…) is the shared
[`gamut-isobmff`](../../crates/gamut-isobmff) primitive — see
[`references/isobmff`](../isobmff/README.md) for that box grammar; this directory covers the
HEIF-specific still-image profile layered on top and the decode oracle.

## Core specifications — paywalled, not vendored

The two normative base specifications are ISO-paywalled and cannot be redistributed here (same
constraint documented in [`references/isobmff`](../isobmff/README.md)):

- **ISO/IEC 23008-12 — HEIF (High Efficiency Image File Format).** Defines still-image items,
  the `hdlr` handler type `"pict"`, the HEIF image-item properties (`ispe`/`pixi`/`colr`/`irot`/
  `imir`/`clap`/`rloc`…), thumbnails/auxiliary/derived items, and — for HEIC — the storage of
  HEVC intra images (`hvc1`/`hev1` items, the `hvcC` configuration property).
- **ISO/IEC 14496-12 — ISO Base Media File Format (ISOBMFF).** The underlying box grammar
  (`Box`/`FullBox`, 4-byte size + 4-character type). Covered by
  [`references/isobmff`](../isobmff/README.md).
- **ISO/IEC 23000-22 — MIAF (Multi-Image Application Format).** Additional interoperability
  constraints on the HEIF profile (essential-property rules, allowed brands).

## Vendored (public domain)

- **Library of Congress — *High Efficiency Image File (HEIF) Format, MPEG-H Part 12*** Format
  Description (FDD): `loc-fdd000525-heif.html`. A U.S. Library of Congress digital-format
  description; the page is marked *"Text is U.S. government work"* (public domain), so it is
  redistributable. Covers file brands (`ftyp`), the `meta`/item model, item properties (`iprp`),
  derived images, and the format's relationship to ISOBMFF and MIAF.
  Source: <https://www.loc.gov/preservation/digital/formats/fdd/fdd000525.shtml> (retrieved 2026-07-12).

## Public references — non-paywalled but not redistributable (cited by link)

- **Nokia HEIF technical documentation** — <https://nokiatech.github.io/heif/technical.html>
  (`technical.html`, `comparison.html`, `examples.html`). The authoritative public narrative of the
  23008-12 container model from the format's originators, including the descriptive/transformative
  item-property split and the derived-image (grid/overlay/identity) constructs. **Not vendored:** the
  pages carry an explicit Nokia copyright notice reserving all rights ("Copying, including
  reproducing [or] storing … requires the prior written consent of Nokia"), so they are linked, not
  copied into the tree.
- **Public still-image box table** — the exact box set and field layouts a HEIF still image uses are
  enumerated in the vendored [AVIF v1.2.0 spec](../avif/v1.2.0.html) (`references/avif`, §2.2/§6/§8.3)
  and, for structure, [AV1-ISOBMFF v1.3.0](../av1/av1-isobmff) (`references/av1`). AVIF is the AV1
  sibling of HEIC and shares the identical container profile — HEIC differs only in carrying an
  `hvcC`/HEVC item where AVIF carries `av1C`/AV1.
- **MP4 Registration Authority** — <https://mp4ra.org> — the public registry of 4-character box
  types and brands used across the ISOBMFF family.

## Codec boundary

The HEIF *container* stops at the opaque codec configuration (`hvcC`) plus the coded samples in
`mdat`/`idat`. HEIC's coded payload is **HEVC intra** (ITU-T H.265 | ISO/IEC 23008-2), which is
*freely available* from the ITU-T (<https://www.itu.int/rec/T-REC-H.265>) and belongs to the codec
references, not this container directory. Only the intra-frame still-image subset is in scope — no
inter-frame/sequence coding.

## Oracle

Conformance for the parser and HEVC-intra decoder is verified differentially against a vendored C
reference decoder, mirroring the libavif/dav1d oracle that [`gamut-avif`](../../crates/gamut-avif)
uses (a `tooling/…-oracle` crate over a `third_party/` git submodule):

- **libheif — <https://github.com/strukturag/libheif>.** The de-facto ISO/IEC 23008-12 HEIF/HEIC
  decoder (BSD/LGPL); it parses the container and decodes HEVC items via **libde265**. Primary
  decode-conformance oracle: a gamut-authored image is round-tripped through libheif's parser and the
  decoded pixels are compared, exercising every box and property the crate reads.
- **Nokia HEIF reference software — <https://github.com/nokiatech/heif>.** The originators'
  conformant reader/writer and a secondary source of sample HEIC content for fixtures.

Deferred and out-of-scope boxes/items are tracked in the consuming crate's `STATUS.md`.
