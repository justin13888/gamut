# AVIF (AV1 Image File Format)

Reference for **`gamut-avif`** — the AVIF still-image encoder. AVIF is a layering, not a new codec:
an AV1 intra-frame bitstream carried as an item in an ISOBMFF/MIAF container. This crate is the glue;
the substantive specs live with the layers it composes:

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

## Conformance

`gamut-avif/tests/decode_roundtrip.rs` is a differential oracle: it encodes an image, has the
vendored **libavif** (with a **dav1d** backend, from `third_party/libavif` + `third_party/dav1d` via
`tooling/libavif-oracle`) parse the container and decode it, and checks the result — bit-exact to the
source for lossless, bit-exact to the AV1 reconstruction for lossy, and that `irot`/`imir` are
accepted as essential properties. The container layout is pinned independently by `gamut-isobmff`'s
`read(&write) == img` round-trip and by `gamut-avif`'s own parse-back unit tests.

The wrapped **AV1 bitstream** is validated one layer down by `gamut-av1` against **libaom** — the AV1
reference codec, the definitive oracle — with `dav1d` corroborating (see
[`references/av1`](../av1/README.md)). No system-installed codec is used at any layer.
