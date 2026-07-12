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

## Decode-surface layouts

The layouts `gamut-heic` reads (issue #238 — container parsing only; the HEVC bitstream decode is
issue #18). Every table below was cross-checked against at least two independent public sources;
per-field provenance is noted. Bit fields are big-endian, MSB-first, as in ISOBMFF.

### 1. `hvcC` — HEVCDecoderConfigurationRecord (ISO/IEC 14496-15 §8.3.3.1)

The body of the `hvcC` item property. Fixed 23-byte header, then `numOfArrays` parameter-set
arrays. All `reserved` fields are written as all-ones and **must be ignored** on read (a reader must
not reject non-conforming reserved bits).

| Field | Bits | Notes |
|---|---|---|
| `configurationVersion` | 8 | = 1 |
| `general_profile_space` | 2 | HEVC `profile_tier_level()` copy |
| `general_tier_flag` | 1 | |
| `general_profile_idc` | 5 | 1 = Main, 2 = Main 10, 3 = Main Still Picture, 4 = Rext |
| `general_profile_compatibility_flags` | 32 | |
| `general_constraint_indicator_flags` | 48 | |
| `general_level_idc` | 8 | |
| `reserved` = `1111`b | 4 | |
| `min_spatial_segmentation_idc` | 12 | |
| `reserved` = `111111`b | 6 | |
| `parallelismType` | 2 | 0 = unknown/mixed, 1 = slice, 2 = tile, 3 = WPP |
| `reserved` = `111111`b | 6 | |
| `chroma_format_idc` | 2 | 0 = mono, 1 = 4:2:0, 2 = 4:2:2, 3 = 4:4:4 |
| `reserved` = `11111`b | 5 | |
| `bit_depth_luma_minus8` | 3 | luma bit depth − 8 |
| `reserved` = `11111`b | 5 | |
| `bit_depth_chroma_minus8` | 3 | chroma bit depth − 8 |
| `avgFrameRate` | 16 | 0 = unspecified (still image) |
| `constantFrameRate` | 2 | |
| `numTemporalLayers` | 3 | |
| `temporalIdNested` | 1 | |
| `lengthSizeMinusOne` | 2 | NAL length-prefix size − 1 (⇒ 1/2/4-byte prefixes); governs §2 below |
| `numOfArrays` | 8 | |

Then `numOfArrays` repetitions of:

| Field | Bits | Notes |
|---|---|---|
| `array_completeness` | 1 | 1 ⇒ array holds *all* NALUs of `NAL_unit_type` and none appear inband |
| `reserved` = `0`b | 1 | (note: `0`, unlike the all-ones reserveds above) |
| `NAL_unit_type` | 6 | typically 32 (VPS), 33 (SPS), 34 (PPS), 39/40 (SEI) — see §3 |
| `numNalus` | 16 | NALUs in this array |

then `numNalus` repetitions of:

| Field | Bits | Notes |
|---|---|---|
| `nalUnitLength` | 16 | length of the following `nalUnit` in bytes |
| `nalUnit` | 8 × `nalUnitLength` | raw NAL unit (header + RBSP, no start code) |

*Verified:* the reserved-bit constants match l-smash's writer (`min_spatial_segmentation_idc | 0xF000`,
`parallelismType | 0xFC`, `chromaFormat | 0xFC`, `bitDepthLumaMinus8 | 0xF8`, `bitDepthChromaMinus8 | 0xF8`)
and the field order/widths match FFmpeg's `HEVCDecoderConfigurationRecord` and the wangyoucao577
medialib Go struct.

### 2. Coded `hvc1`/`hev1` item payload (ISO/IEC 14496-15 §8.3.2; ISO/IEC 23008-12 §7 / Annex B.2)

An HEVC image item's coded data (located via `iloc`) is a **length-prefixed NAL unit stream** — the
14496-15 *sample* structure applied to an item. Each NAL unit is preceded by a length field of
`lengthSizeMinusOne + 1` bytes (1/2/4; 4 in practice), and the NAL units are concatenated with no
Annex-B start codes:

```
repeat until end of item extent:
    unsigned int((lengthSizeMinusOne+1)*8)  NALUnitLength
    bit(8 * NALUnitLength)                  NALUnit          // §3 header + RBSP
```

- **`hvc1`** — parameter sets (VPS/SPS/PPS) live **only** in `hvcC`; they never appear inband in the
  item payload. `array_completeness` is expected to be 1.
- **`hev1`** — parameter sets may appear in `hvcC` **and/or** inband in the item payload (mirrors the
  `avc1`/`avc3` split in 14496-15). A reader must scan the payload for inband parameter sets.

**Annex-B conversion:** a raw HEVC decoder (ITU-T H.265 Annex B) expects `00 00 01` / `00 00 00 01`
start-coded NAL units. To feed one, replace each `NALUnitLength` prefix with a start code (and, for
`hev1`, prepend the `hvcC` parameter sets if absent inband). *Verified* against 14496-15 §8.3.2/§8.4
and the AVCC↔Annex-B convention shared with H.264.

### 3. HEVC NAL unit header (ITU-T H.265 §7.3.1.2, Table 7-1)

Two bytes at the start of every NAL unit (`nalUnit` / `NALUnit` above). Freely downloadable:
<https://www.itu.int/rec/T-REC-H.265>.

| Field | Bits | Notes |
|---|---|---|
| `forbidden_zero_bit` | 1 | = 0 |
| `nal_unit_type` | 6 | see table below |
| `nuh_layer_id` | 6 | 0 for the base layer (still-image items are single-layer) |
| `nuh_temporal_id_plus1` | 3 | TemporalId + 1; = 1 for a still image |

NAL types the container layer must classify (ITU-T H.265 Table 7-1):

| `nal_unit_type` | Name | Role |
|---|---|---|
| 16 | `BLA_W_LP` | IRAP (VCL) |
| 17 | `BLA_W_RADL` | IRAP (VCL) |
| 18 | `BLA_N_LP` | IRAP (VCL) |
| 19 | `IDR_W_RADL` | IRAP (VCL) |
| 20 | `IDR_N_LP` | IRAP (VCL) |
| 21 | `CRA_NUT` | IRAP (VCL) |
| 22 | `RSV_IRAP_VCL22` | reserved IRAP |
| 23 | `RSV_IRAP_VCL23` | reserved IRAP |
| 32 | `VPS_NUT` | video parameter set |
| 33 | `SPS_NUT` | sequence parameter set |
| 34 | `PPS_NUT` | picture parameter set |
| 39 | `PREFIX_SEI_NUT` | SEI |
| 40 | `SUFFIX_SEI_NUT` | SEI |

**Still-image constraint:** a HEIF HEVC image item carries **intra-coded / IRAP content only** — the
coded picture is one of the IRAP types 16..=23 (BLA/IDR/CRA), so the item is independently decodable
with no inter-picture prediction. *Verified* against H.265 Table 7-1, the GStreamer `GstH265NalUnitType`
enum, and RFC 7798 §1.1.4.

### 4. Derived image items (ISO/IEC 23008-12 §6.6.2)

A derived image item synthesises an output image from the items its `dimg` reference lists; it has no
coded pixels. Three kinds are in scope:

- **`iden` — identity** (§6.6.2.1): **no item payload**. References exactly one image via a single
  `dimg` entry; the output is that image with its own associated transformative properties
  (`clap`/`irot`/`imir`, §5) applied. (Nokia describes it as "cropping and/or rotation … imposed
  through the respective transformative properties.")
- **`grid`** (§6.6.2.2): tile matrix reassembly. Payload layout (`ImageGrid`) is documented in
  [`references/isobmff`](../isobmff/README.md#derived-image-payloads); tiles are the row-major `dimg`
  targets.
- **`iovl` — ImageOverlay** (§6.6.2.3): composits the `dimg`-referenced items onto a filled canvas.
  Item payload:

| Field | Bits | Notes |
|---|---|---|
| `version` | 8 | = 0 |
| `flags` | 8 | bit 0 selects `FieldLength` |
| `canvas_fill_value[0]` (R) | 16 | canvas background, unsigned |
| `canvas_fill_value[1]` (G) | 16 | unsigned |
| `canvas_fill_value[2]` (B) | 16 | unsigned |
| `canvas_fill_value[3]` (A) | 16 | unsigned |
| `output_width` | `FieldLength` | unsigned; `FieldLength = ((flags & 1) + 1) * 16` ⇒ 16 or 32 |
| `output_height` | `FieldLength` | unsigned |

then, per referenced image `i` (same count and order as the `dimg` reference list):

| Field | Bits | Notes |
|---|---|---|
| `horizontal_offset` | `FieldLength` | **signed** — top-left placement on canvas |
| `vertical_offset` | `FieldLength` | **signed** |

*Verified:* field order/widths, the RGBA fill values, unsigned `output_*` and **signed** offsets all
match libheif's `Box_iovl` / overlay reader; the `FieldLength` rule mirrors `ImageGrid`.

### 5. Transformative property application order (ISO/IEC 23008-12; MIAF constraint)

23008-12 applies an item's associated **transformative** properties in the order they are listed in
that item's `ipma` association array. MIAF (ISO/IEC 23000-22) tightens this for interop: **at most one
each** of `clap`, `irot`, `imir` per item, and — when more than one is present — they are applied in
the fixed order **clean aperture (`clap`) → rotation (`irot`) → mirror (`imir`)**. The AVIF v1.2.0
spec restates this alignment in **§2.2.3 (Clean Aperture Property)**: it defers to "the restrictions
on transformative item property ordering specified in [MIAF]" and additionally anchors the `clap`
origin to (0,0). *Verified* against AVIF §2.2.3 (vendored [`references/avif/v1.2.0.html`](../avif/v1.2.0.html))
and the MIAF-derived ordering language.

### 6. Auxiliary image items (`auxC`) (ISO/IEC 23008-12 §6.5.8; MIAF §7.3.5)

An auxiliary image is linked to its master by an `auxl` item reference (auxiliary → master) and typed
by the `auxC` `AuxiliaryTypeProperty`, whose `aux_type` is a null-terminated URN:

| `aux_type` URN | Meaning | Source |
|---|---|---|
| `urn:mpeg:hevc:2015:auxid:1` | alpha | 23008-12 (HEVC auxiliary), Apple HEIC |
| `urn:mpeg:hevc:2015:auxid:2` | depth | 23008-12 (HEVC auxiliary), Apple HEIC |
| `urn:mpeg:mpegB:cicp:systems:auxiliary:alpha` | alpha | MIAF / CICP (AVIF-style) |
| `urn:mpeg:mpegB:cicp:systems:auxiliary:depth` | depth | MIAF / CICP (AVIF-style) |

Apple's HEICs use the `hevc:2015` URNs for basic alpha/depth, plus proprietary
`urn:com:apple:photo:…:aux:…` URNs (e.g. `…portraiteffectsmatte`, `…hdrgainmap`) for
computational-photography auxiliaries — treat any unrecognised `aux_type` as an opaque, non-displayed
plane. **`prem`** is an item reference (from the premultiplied colour item *to* its alpha auxiliary
item, 23008-12; libheif/libavif agree on this direction), not a property: it signals that the colour
item's values are already premultiplied by that alpha auxiliary.

### 7. Brands (`ftyp`)

Every HEIF still carries the structural brand `mif1`; codec/profile brands and MIAF brands appear
alongside it. In-scope stills vs out-of-scope sequences:

| Brand | Meaning | Scope |
|---|---|---|
| `mif1` | HEIF structural brand (image file format) — present in every still | in |
| `mif2` | structural brand, CICP alpha & depth | in |
| `heic` | HEVC image / image collection (Main-profile-ish) | in |
| `heix` | HEVC image / image collection (Main 10 / extended) | in |
| `heim` | L-HEVC (layered) image | in (single-layer read) |
| `heis` | L-HEVC image | in (single-layer read) |
| `miaf` | MIAF general requirements | in |
| `MiHB` / `MiHA` / `MiHE` | MIAF HEVC Basic / Advanced / Extended profile | in |
| `avif` | AV1 image (sibling format) | (gamut-avif) |
| `msf1` | HEIF image-sequence structural brand | **out** |
| `hevc` / `hevx` | HEVC image *sequence* | **out** |

*Verified* against MP4RA's brand registry and the LoC FDD (`fdd000525`). Sequence brands
(`msf1`/`hevc`/`hevx`) are out of scope — gamut is image-first (no inter-frame coding).

### 8. Motion-photo container surface (vendor-neutral)

Real-world HEICs from phones carry extra data beyond the still-image box tree. **Issue #238
requirement: the container representation must account for every byte of the file — nothing is
dropped — while vendor-specific semantics stay downstream.** Two shapes occur:

- **(a) Unknown top-level boxes.** Google/Android Motion Photo appends a top-level
  `mpvd` box (Motion Photo Video Data — `aligned(8) class MotionPhotoVideoData extends Box('mpvd') { bit(8) data[]; }`)
  after all the image boxes; its `data[]` is a complete MP4. The parser must preserve unknown
  top-level boxes verbatim rather than error. **`mpvd` is *not* in the MP4RA box registry** (verified
  absent) — it is defined only by the Android Motion Photo spec.
- **(b) Trailing bytes after the last top-level box.** Some vendors (e.g. Samsung) append raw bytes
  *after* the final top-level box — typically a second file (an appended MP4 whose first box is a
  second `ftyp`) plus a proprietary trailer (Samsung's "SEF"). These are not inside any box, so the
  container model must retain the byte range between the end of the last parsed box and EOF.

*Verified:* `mpvd` definition/placement from the Android Motion Photo spec; MP4RA absence confirmed.
The Samsung SEF trailer layout is vendor-proprietary and undocumented publicly — treated as an opaque
retained byte range, not parsed. **(flagged — see caveats.)**

### 9. Metadata items

- **Exif** — an item with `item_type = "Exif"` whose payload is an `ExifDataBlock` (23008-12 §A.2.1):

| Field | Bits | Notes |
|---|---|---|
| `exif_tiff_header_offset` | 32 | byte count from the end of this field to the TIFF header; usually 0 |
| `exif_payload` | 8 × N | the Exif block, beginning (after the offset) with the TIFF header `II`/`MM` |

  A reader skips `exif_tiff_header_offset` bytes to locate the `II`(0x4949) / `MM`(0x4D4D) TIFF header,
  then parses via the shared `gamut-exif` primitive. *Verified* against the TIFF-header-inside-HEIC
  handling in exif-py/ExifTool; **the 4-byte-prefix detail is not confirmable from a second primary
  spec source (flagged).**
- **XMP** — a `mime` item (`infe` `item_type = "mime"`) with `content_type = "application/rdf+xml"`;
  the payload is the raw XMP packet (no offset prefix), parsed via `gamut-xmp`.
- **`cdsc` direction** — the metadata item is the reference **source** and the described image item is
  the **target**: an `iref` of type `cdsc` runs *from* the Exif/XMP item *to* the image item it
  describes.

## Codec boundary

The HEIF *container* stops at the opaque codec configuration (`hvcC`) plus the coded samples in
`mdat`/`idat`. Parsing the `hvcC` record (§1), splitting the item payload into length-prefixed NAL
units (§2), and reading the NAL unit header to *classify* each NALU (§3 — parameter set vs SEI vs
IRAP slice) are all **container scope** (issue #238): the reader needs them to demux parameter sets,
validate the still-image constraint, and hand a decoder-ready NAL stream downstream. What the
container does **not** do is interpret the RBSP payloads — slice-segment/CTU decoding, entropy
decoding, and pixel reconstruction are **codec scope** (issue #18). HEIC's coded payload is **HEVC
intra** (ITU-T H.265 | ISO/IEC 23008-2), which is *freely available* from the ITU-T
(<https://www.itu.int/rec/T-REC-H.265>) and belongs to the codec references, not this container
directory. Only the intra-frame still-image subset is in scope — no inter-frame/sequence coding.

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
