# gamut-icc — ICC profile implementation status

Part of the **image metadata primitives** campaign (GitHub issue #34). Implements the ICC profile
format (`references/icc`, ICC.1:2022 = ISO 15076-1) as a parser + serializer.

**Keystone:** the multi-dimensional transform tag types — `lutAToB`/`lutBToA` (`mAB `/`mBA `) and the
legacy `lut8`/`lut16` — which carry the matrix → curve → CLUT → curve pipeline that defines
device↔PCS conversion.

**Oracle:** differential vs **Little-CMS (lcms2)** (dev-only FFI, `tooling/lcms2-oracle`) — gamut-icc
decodes lcms-synthesized profiles to the same values lcms reports, and lcms re-opens gamut-icc's
serialization as an equivalent profile.

## Phases

| Phase | Spec § | Scope | Status |
| ----- | ------ | ----- | ------ |
| P1 | — | Scaffold: crate, workspace wiring, docs, data-model skeleton | ✅ |
| P2 | §7.2–7.3 | Header parse (all fields) + tag table | ✅ |
| P3 | §10 | Simple element types: `XYZType`, `curveType`, `parametricCurveType`, `textType`, `multiLocalizedUnicodeType` | ✅ |
| P4 | §9 | Matrix/TRC (shaper) profiles: `rXYZ`/`gXYZ`/`bXYZ` + `rTRC`/`gTRC`/`bTRC` + `wtpt`/`chad`/`desc`/`cprt` | ✅ |
| P5 | §10 | **Keystone** — LUT transform types: `lut8`/`lut16`/`lutAToB`/`lutBToA` | ✅ |
| P6 | §7 | Writer/serialize + round-trip; `size` and profile-ID (MD5) recomputation | ✅ |
| P7 | — | v2 legacy quirks (`textDescriptionType`) | ✅ |
| P8 | — | lcms2 differential oracle gate | ✅ |

## Modelled element types

Decoded semantically: `XYZType`, `curveType`, `parametricCurveType` (function types 0–4), `textType`,
`multiLocalizedUnicodeType`, `textDescriptionType` (v2), `dateTimeType`, `signatureType`,
`s15Fixed16ArrayType`, `lut8Type`, `lut16Type`, `lutAToBType`, `lutBToAType`, `namedColor2Type`.

Every other element type is preserved verbatim as `TagData::Raw` and round-trips byte-for-byte, so
no profile is rejected for carrying an unmodelled tag.

## Deferred

- **Applying transforms** (a CMM): gamut-icc parses and serializes profiles; evaluating a profile's
  device↔PCS conversion is out of scope. Curve/`XYZ`/matrix values expose `to_f64`/`eval` accessors
  as the seam.
- **`gamut-color` integration**: building runnable transforms (matrix/TRC → pipeline, `chad`
  application) belongs in `gamut-color` (dependency direction `gamut-color → gamut-icc`), not here.
- **Secondary element types** (`chromaticityType`, `measurementType`, `viewingConditionsType`,
  `multiProcessElementsType`, …): preserved as `Raw` rather than decoded.
- **iccMAX (`ICC.2:2019`, profile version 5)**: out of scope. iccMAX is a separate, parallel
  next-generation format (spectral PCS, `multiProcessElementsType`, ~20 new tag types), *not* an
  extension of the ICC.1 format this crate targets; the real-world profiles embedded in images are
  all ICC.1 v2/v4, and the lcms2 oracle does not implement iccMAX. See
  [`references/icc/README.md`](../../references/icc/README.md).
