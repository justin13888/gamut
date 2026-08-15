# ICC colour management module — CMM behaviour references (issue #323)

Reference material for the `gamut-cmm` crate — the colour management module that builds and
applies colour transforms from the ICC profiles parsed by `gamut-icc`.

The profile *format* is specified by **ICC.1:2022**, vendored under
[`references/icc/`](../icc/README.md) together with the legacy v2 edition; the numeric encodings
this CMM consumes (fixed-point formats, PCS Lab/XYZ encodings) are documented there and in
[`references/color/`](../color/README.md). The format specification, however, pins down **data
layouts, not CMM behaviour**: interpolation methods, clamping, evaluation precision, and much of
intent handling are explicitly left to the CMM implementation.

## Behavioural oracle — Little-CMS

`gamut-cmm`'s behavioural oracle is therefore **Little-CMS (lcms2)**, the de-facto reference
CMM: differential tests link it dev-only through `tooling/lcms2-oracle`, built from the
`third_party/lcms2` submodule (**lcms2 2.19**). Where observable behaviour is unspecified by
ICC.1 — e.g. clamp semantics for NaN and out-of-range samples, CLUT interpolation — `gamut-cmm`
matches lcms2 and documents the choice at the API (see `crates/gamut-cmm`).

## Vendored primary sources

None yet — this table is filled by the phases that transcribe constants (#329 adds the
rendering-intent and black-point-compensation sources).

| file | source |
|------|--------|

## Not vendored (paywalled — constants transcribed inline by the PRs that need them)

- **ISO 18619:2015** (Image technology colour management — black point compensation) — ISO,
  paywalled. The BPC algorithm implemented in #329.
- **ISO 12640-3** (Graphic technology — prepress digital data exchange — Part 3: CIELAB standard
  colour image data, SCID) — ISO, paywalled.
- **Kasson, Nin, Plouffe & Hafner, "Performing color space conversions with three-dimensional
  linear interpolation", *J. Electronic Imaging* 4(3), 1995** — paywalled; the tetrahedral
  interpolation primary source for #326.
