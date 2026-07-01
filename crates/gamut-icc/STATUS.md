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
| P9 | §10 | **Full §10 coverage** — every remaining element type decoded (see below) | ✅ |
| P10 | §8 | Profile-class conformance validation (`IccProfile::validate`) | ✅ |

## Modelled element types

**Every ICC.1:2022 §10 element type is decoded semantically:** `XYZType`, `curveType`,
`parametricCurveType` (function types 0–4), `textType`, `multiLocalizedUnicodeType`,
`textDescriptionType` (v2), `dateTimeType`, `signatureType`, `s15Fixed16ArrayType`, `lut8Type`,
`lut16Type`, `lutAToBType`, `lutBToAType`, `namedColor2Type`, `chromaticityType`, `cicpType`,
`measurementType`, `viewingConditionsType`, `dataType`, `colorantOrderType`, `colorantTableType`,
`u16Fixed16ArrayType`, `uInt8/16/32/64ArrayType`, `profileSequenceDescType`,
`profileSequenceIdentifierType`, `responseCurveSet16Type`, and `dictType`.

Any element type *not* defined in ICC.1:2022 §10 (e.g. iccMAX's `multiProcessElementsType`, or
private/unregistered types) is preserved verbatim as `TagData::Raw` and round-trips byte-for-byte,
so no profile is rejected for carrying an unmodelled tag.

## Deferred

- **Applying transforms** (a CMM): gamut-icc parses and serializes profiles; evaluating a profile's
  device↔PCS conversion is out of scope. Curve/`XYZ`/matrix values expose `to_f64`/`eval` accessors
  as the seam.
- **`gamut-color` integration**: building runnable transforms (matrix/TRC → pipeline, `chad`
  application) belongs in `gamut-color` (dependency direction `gamut-color → gamut-icc`), not here.
- **`multiProcessElementsType`** (`mpet`, and the `D2Bx`/`B2Dx` transform tags that use it): the
  v4/iccMAX general-purpose processing pipeline is preserved as `Raw` rather than decoded. Every
  other §10 element type is now modelled (see above).
- **iccMAX (`ICC.2:2019`, profile version 5)**: out of scope. iccMAX is a separate, parallel
  next-generation format (spectral PCS, `multiProcessElementsType`, ~20 new tag types), *not* an
  extension of the ICC.1 format this crate targets; the real-world profiles embedded in images are
  all ICC.1 v2/v4, and the lcms2 oracle does not implement iccMAX. See
  [`references/icc/README.md`](../../references/icc/README.md).
