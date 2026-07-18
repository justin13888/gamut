# AVIF (AV1 Image File Format)

Reference for **`gamut-avif`** — the AVIF still-image encoder and **container decoder**
(issue #250). AVIF is a layering, not a new codec: an AV1 intra-frame bitstream carried as an item
in an ISOBMFF/MIAF container. This crate is the glue; the substantive specs live with the layers
it composes:

- **AV1 bitstream** (the coded `mdat` payload + the `av1C` configuration record) — see
  [`references/av1`](../av1), including AV1-ISOBMFF v1.3.0 for the `av01` item type and the
  `AV1CodecConfigurationRecord` (§2.3) stamped into an `ipco` property.
- **ISOBMFF / HEIF / MIAF container** (`ftyp`/`meta`/`iloc`/`iprp`/`mdat`, the
  `ispe`/`pixi`/`colr`/`irot`/`imir` properties, essential-property rules) — see
  [`references/isobmff`](../isobmff).
- **CICP colour code points** (the `colr` `nclx` values and the AV1 `color_config`) — ITU-T H.273,
  via [`references/color`](../color).

## Vendored

- **AV1 Image File Format (AVIF) v1.2.0** — [`v1.2.0.html`](./v1.2.0.html). The public AVIF
  specification: file brands (§8.3), the still-image item structure and property requirements
  (§2.2), and the minimum box set a single image uses (§9.1.1). It is publicly redistributable
  (unlike the ISO base specs — see `references/isobmff`), so it is vendored in full.

The identity-matrix 8-bit path `gamut-avif` emits today stamps `colr` `nclx` =
`(colour_primaries 1 = BT.709, transfer_characteristics 13 = sRGB, matrix_coefficients 0 = Identity,
full_range)`, matching the AV1 sequence header by construction (AVIF v1.2.0 §2.2; AV1-ISOBMFF §2.3.4
requires the two to agree, and `matrix_coefficients 0` requires 4:4:4 full range).

## Quality → AV1 quantizer mapping

The only numeric mapping `gamut-avif` itself defines (everything else is delegated): the public
`0..=100` `quality` factor → the AV1 `base_q_idx` passed to `gamut-av1`. There is no single
standard for this — libaom/libavif map a quality/`cq-level` to the quantizer through encoder-internal
tables — so gamut defines its own simple, monotonic mapping (finer, metric- or size-targeting rate
control is future work, tracked in `STATUS.md`). This mapping — including the silent clamp of
`quality > 100` to `100` — is a **frozen `gamut-avif` v1 contract**:

```text
lossless()  ->  base_q_idx = 0          # AV1 CodedLossless (Walsh–Hadamard); bit-exact
lossy(q)    ->  base_q_idx = max(1, (100 - clamp(q, 0, 100)) * 255 / 100)   # integer division
```

`base_q_idx` runs `0..=255` (AV1 §5.9.12), higher = coarser quantization. Higher `quality` therefore
yields a lower index. Endpoints: `lossy(100) -> 1`, `lossy(50) -> 127`, `lossy(0) -> 255`. The floor
of `1` keeps `lossy(_)` on the lossy DCT pipeline; `base_q_idx 0` (the lossless path) is reserved for
`lossless()`/the default, so the lossless and lossy modes never alias.

## The decode surface (issue #250)

The read side implements the **AV1 Image Item Data** constraints normatively (all enforced as
errors before a payload reaches the pluggable `Av1StillDecoder`):

- AVIF v1.2.0 §2.1 — the item data is a sync-sample temporal unit with **exactly one** Sequence
  Header OBU;
- AV1-ISOBMFF §2.4 — every OBU carries `obu_has_size_field = 1` except (optionally) the last,
  which then fills the remainder; `OBU_TILE_LIST` SHALL NOT appear; the sync-sample Random Access
  Point rules (a Sequence Header OBU before the first frame-bearing OBU; the first frame a shown
  key frame, checked from the fixed leading `uncompressed_header()` bits, AV1 §5.9.2).

SHOULD-level shapes are tolerated (temporal delimiters, padding, redundant frame headers, a
sequence header repeated in `configOBUs`, `still_picture`/`reduced_still_picture_header` = 0), and
the §2.3.4 rule that a `configOBUs` sequence header match the payload's is left to the decoder.

The RGBA presentation path supports the 8-bit still-image colour cases — identity `mc=0`
(requiring 4:4:4), BT.601 `mc=5/6`, `mc=2` unspecified (treated as BT.601, matching libavif's
fallback), and monochrome — with nearest co-sited chroma upsampling. A **missing `colr` defaults
to BT.601 limited range** (the posture `gamut-heic` documents for HEIF): AVIF technically defers
to the AV1 sequence header's `color_config`, which the container layer deliberately does not
parse; callers needing sequence-header CICP use the planar surface. `imir` is applied with the
ISO/IEC 23008-12:**2022** §6.5.12 axis semantics (axis 0 exchanges top/bottom, axis 1 left/right
— the reading libheif and libavif implement).

## Conformance

`gamut-avif/tests/decode_roundtrip.rs` is a differential oracle: it encodes an image, has the
vendored **libavif** (with a **dav1d** backend, from `third_party/libavif` + `third_party/dav1d` via
`tooling/libavif-oracle`) parse the container and decode it, and checks the result — bit-exact to the
source for lossless, bit-exact to the AV1 reconstruction for lossy, and that `irot`/`imir` are
accepted as essential properties. The container layout is pinned independently by `gamut-isobmff`'s
`read(&write) == img` round-trip and by `gamut-avif`'s own parse-back unit tests.

The decode surface has its own differential suite, `gamut-avif/tests/conformance.rs`, over the
libavif conformance corpus committed in `third_party/libavif/tests/data`: libavif's parse
(`introspect`) pins container structure, CICP, transforms, and the ICC/Exif/XMP payloads
byte-exact; libavif's own RGBA presentation (`decode_rgba`, nearest-neighbour upsampling on both
sides) bounds the colour path within conversion rounding with alpha exact; and **dav1d plugged
directly into the `Av1StillDecoder` seam** proves the planar pipeline bit-exact against both the
raw codestream and libavif's independent container decode. Self-encoded lossless files decode
back bit-exact end to end through the same seam.

The wrapped **AV1 bitstream** is validated one layer down by `gamut-av1` against **libaom** — the AV1
reference codec, the definitive oracle — with `dav1d` corroborating (see
[`references/av1`](../av1/README.md)). No system-installed codec is used at any layer.
