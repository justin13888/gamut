# gamut-icc — ICC profile implementation status

Part of the **image metadata primitives** campaign (GitHub issue #34). Implements the ICC profile
format (`references/icc`, ICC.1:2022 = ISO 15076-1) as a parser + serializer. Delivered as a stack
of small, individually-reviewable PRs onto the `feat/metadata-primitives` integration branch; each
PR is independently green (`just test`/`lint`/`format-check`/`coverage` ≥ 80%).

**Keystone:** the multi-dimensional transform tag types — `lutAToB`/`lutBToA` (`mAB `/`mBA `) and the
legacy `lut8`/`lut16`. These carry the matrix → curve → CLUT → curve pipeline that defines device↔PCS
conversion; everything else (header, XYZ/curve/text tags) is comparatively mechanical.

**Oracle:** differential vs **Little-CMS (lcms2)** (dev-only FFI) — `gamut-icc` parse + re-serialize
must round-trip a profile that lcms2 accepts as equivalent.

## Phases

| Phase | Spec § | Scope | Status |
| ----- | ------ | ----- | ------ |
| P1 | — | Scaffold: crate, workspace wiring, docs, region-free data-model skeleton | ✅ in progress |
| P2 | §7.2–7.3 | Header parse (all fields) + tag table | ☐ |
| P3 | §10 | Simple element types: `XYZType`, `curveType`, `parametricCurveType`, `textType`, `multiLocalizedUnicodeType` | ☐ |
| P4 | §9 | Matrix/TRC (shaper) profiles: `rXYZ`/`gXYZ`/`bXYZ` + `rTRC`/`gTRC`/`bTRC` + `wtpt`/`chad`/`desc`/`cprt` | ☐ |
| P5 | §10 | **Keystone** — LUT transform types: `lut8`/`lut16`/`lutAToB`/`lutBToA` | ☐ |
| P6 | §7 | Writer/serialize + round-trip; `size` and profile-ID (MD5) recomputation | ☐ |
| P7 | — | v2 legacy quirks (`textDescriptionType`, v2 LUT layouts) + edge profiles | ☐ |
| P8 | — | lcms2 differential oracle gate | ☐ |
