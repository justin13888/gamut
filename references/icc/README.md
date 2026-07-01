# ICC profiles (International Color Consortium)

Reference specifications for the `gamut-icc` crate. The ICC publishes every current and superseded
edition, freely, from its specification index: <https://www.color.org/specification/index.xalter>.

## Authoritative editions (vendored)

- `icc.1-2022-05.pdf` — **ICC.1:2022 (Profile version 4.4.0.0)** — the current ICC profile format
  specification, technically equivalent to **ISO 15076-1**, and the edition `gamut-icc` targets.
  Published freely by the International Color Consortium:
  <https://www.color.org/specification/ICC.1-2022-05.pdf>.
- `icc.1-2001-04.pdf` — **ICC v2 (ICC.1:2001-04, a revision of ICC.1:1998-09)** — the still-ubiquitous
  legacy profile version; supported for reading, since the overwhelming majority of profiles embedded
  in real images are v2. Published freely by the ICC (`ICC_Minor_Revision_for_Web.pdf`):
  <https://www.color.org/icc_specs2.xalter>.

An ICC profile is a self-describing binary blob: a 128-byte header, a tag table, and tag element
data — independent of any IFD/XML structure, so `gamut-icc` depends only on `gamut-core`.

## Not implemented — iccMAX (ICC.2:2019)

`ICC.2:2019` (**iccMAX**, profile version 5) — <https://www.color.org/specification/ICC.2-2019.pdf>
— is **out of scope** and deliberately not implemented. iccMAX is not an extension or superset of
the ICC.1 profile format vendored here; it is a **separate, parallel** next-generation format aimed
at spectral and high-end colour workflows, introducing a distinct v5 header, a spectral PCS,
`multiProcessElementsType` with a programmable calculator element, and roughly twenty new tag types.

Two facts make it the wrong target for this crate:

- **The real-world profiles `gamut-icc` exists to read are all ICC.1 v2/v4.** Every profile embedded
  in a camera JPEG, a PNG `iCCP` chunk, a TIFF/DNG, or a WebP/AVIF is an ICC.1 profile; iccMAX is
  confined to specialist pipelines and is essentially never seen in image files.
- **Our conformance oracle cannot validate it.** The vendored oracle is Little-CMS (lcms2), which
  implements ICC.1 only. iccMAX has its own separate reference engine (the ICC's `RefIccMAX` /
  `DemoIccMAX` project), which is not vendored here.

Should iccMAX support ever be warranted it would be a separate effort with its own reference engine,
not a change to the ICC.1 parser documented below.

## Conformance

Differential oracle against **Little-CMS (lcms2)** (C FFI, `tooling/lcms2-oracle`) for parse +
re-serialize equivalence; see [`gamut-icc/STATUS.md`](../../crates/gamut-icc/STATUS.md).

## Formulas implemented by `gamut-icc`

The numeric encodings and closed forms the crate decodes/evaluates, with their spec sections. All
multi-byte values are big-endian.

### Fixed-point numbers (§4)

| Type | On disk | Value |
| ---- | ------- | ----- |
| `s15Fixed16Number` (§4.6) | signed `i32` | `raw / 65536` (e.g. `0x0001_0000` = 1.0, `0xFFFF_0000` = -1.0) |
| `u16Fixed16Number` (§4.7) | unsigned `u32` | `raw / 65536` |
| `u8Fixed8Number` (§4.5) | unsigned `u16` | `raw / 256` |

`XYZNumber` (§4.14) is three `s15Fixed16`; the header PCS illuminant is the D50 value
`(0.9642, 1.0, 0.8249) ≈ (0x0000_F6D6, 0x0001_0000, 0x0000_D32D)`. `dateTimeNumber` (§4.2) is six
`u16` (year, month, day, hour, minute, second, UTC).

### Profile ID — MD5 (§7.2.18)

The 16-byte profile ID is the MD5 of the *entire* profile with three header fields set to zero
first: **profile flags** (bytes 44–47), **rendering intent** (64–67), and **profile ID** itself
(84–99). The `size` field is left as written.

### Tone curves

`curveType` (§10.6) by entry count `n`: `n = 0` → identity `Y = X`; `n = 1` → gamma
`Y = X^g` with `g` a `u8Fixed8`; `n ≥ 2` → a uniform table over `[0, 1]`, samples `u16 / 65535`,
linearly interpolated.

`parametricCurveType` (§10.18), parameters in order `g, a, b, c, d, e, f`:

| Type | Function |
| ---- | -------- |
| 0 | `Y = X^g` |
| 1 | `Y = (aX + b)^g` for `X ≥ -b/a`, else `0` |
| 2 | `Y = (aX + b)^g + c` for `X ≥ -b/a`, else `c` |
| 3 | `Y = (aX + b)^g` for `X ≥ d`, else `cX` |
| 4 | `Y = (aX + b)^g + e` for `X ≥ d`, else `cX + f` |

### LUT transforms (§10.10–10.13)

`lut8Type`/`lut16Type` apply, in order: 3×3 matrix → input tables → CLUT → output tables. CLUT
samples normalize as `value / 255` (lut8) or `value / 65535` (lut16).

`lutAToBType` (`mAB `, device→PCS) applies A-curves → CLUT → M-curves → matrix(3×3 + 3 offset) →
B-curves; `lutBToAType` (`mBA `, PCS→device) reverses this: B-curves → matrix → M-curves → CLUT →
A-curves. Every stage but the B-curves is optional, signalled by a zero offset in the element
header; sub-elements are 4-byte aligned.

### Measurement & signalling elements

- `chromaticityType` (`chrm`, §10.2): a `u16` channel count, a `u16` phosphor/colorant type
  (Table 31: `0` explicit, `1`–`6` = BT.709-2 / SMPTE RP145 / EBU 3213-E / P22 / P3 / BT.2020), then
  one `u16Fixed16` `(x, y)` pair per channel.
- `cicpType` (`cicp`, §10.3): four `uInt8` ITU-T H.273 (ISO/IEC 23091-2) code points —
  `ColourPrimaries`, `TransferCharacteristics`, `MatrixCoefficients` (0 for RGB/XYZ),
  `VideoFullRangeFlag`. gamut-icc stores the raw code points; interpretation lives in `gamut-color`.
- `measurementType` (`meas`, §10.14): standard-observer code (Table 50) · backing `XYZNumber` ·
  geometry code (Table 51) · flare `u16Fixed16` (Table 52) · standard-illuminant code (Table 53).
- `viewingConditionsType` (`view`, §10.30): illuminant `XYZNumber` · surround `XYZNumber` ·
  illuminant-type code (shared with `measurementType`), all in un-normalized cd/m² CIEXYZ.
- `dataType` (`data`, §10.7): a `uInt32` flag (`0` ASCII / `1` binary) followed by
  `element size − 12` payload bytes, preserved verbatim.

### Colorant & array elements

- `colorantOrderType` (`clro`, §10.4): a `uInt32` count, then that many `uInt8` colorant numbers in
  laydown order. `colorantTableType` (`clrt`, §10.5): a `uInt32` count, then per colorant a 32-byte
  NUL-terminated 7-bit-ASCII name and three `uInt16` PCS values.
- The generic array types decode into vectors sized from the tag length (`(size − 8) / width`):
  `u16Fixed16ArrayType` (`uf32`, §10.25) · `uInt8ArrayType` (`ui08`, §10.29) ·
  `uInt16ArrayType` (`ui16`, §10.26) · `uInt32ArrayType` (`ui32`, §10.27) ·
  `uInt64ArrayType` (`ui64`, §10.28) — the `s15Fixed16ArrayType` (`sf32`, §10.22, used by `chad`)
  already had a decoder.

### Profile-sequence & response-curve elements

- `profileSequenceDescType` (`pseq`, §10.19): a `uInt32` count, then per entry the component
  profile's manufacturer/model/attributes/technology followed by two **self-delimiting** embedded
  descriptions (`multiLocalizedUnicodeType` in v4, `textDescriptionType` in v2). They carry no length
  prefix, so each is walked by recomputing its own serialized length from its internal tables.
- `profileSequenceIdentifierType` (`psid`, §10.20): a `uInt32` count, an 8-byte `(offset, size)`
  positions table, then 4-byte-aligned structures of a 16-byte profile ID + an embedded
  `multiLocalizedUnicodeType`. Offsets are relative to the element start.
- `responseCurveSet16Type` (`rcs2`, §10.21): `uInt16` channel count `n` and measurement-type count
  `m`, an `m`-entry `uInt32` offset table, then per measurement type a curve structure — a
  measurement-unit signature, `n` per-channel `uInt32` counts, `n` PCSXYZ `XYZNumber`s, then the
  `response16Number` arrays (a `uInt16` device code, a reserved `uInt16`, and an `s15Fixed16` value).
