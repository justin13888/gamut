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
  interpolation primary source for #326, transcribed below in the concrete form lcms2
  implements (`TetrahedralInterpFloat`, credited in-source to "Sakamoto's algorithm").

## Transcription: tetrahedral CLUT interpolation (#326)

From `third_party/lcms2` (lcms2 2.19), `src/cmsintrp.c:620-724` (`TetrahedralInterpFloat`);
the paywalled origin of the decomposition is Kasson–Nin–Plouffe–Hafner 1995 (above).
Implemented by `gamut-cmm`'s `ClutTable` (`crates/gamut-cmm/src/clut.rs`).

**Cell mapping** (`cmsintrp.c:223-227, 638-655`). Per input channel: `fclamp` maps NaN and
every value below `1e-9` (all negatives included) to `0.0` and everything above `1.0` to
`1.0`; then `px = fclamp(in) · Domain` with `Domain = gridPoints − 1`, lower node `x0 = ⌊px⌋`,
fraction `rx = px − x0`, and the upper node equals the lower **when `fclamp(in) >= 1.0`**
(otherwise `x0 + 1`) — the edge rule that keeps the top grid plane in bounds.

**Decomposition.** The unit cube splits into six tetrahedra selected by ordering the three
fractions; the interpolant is `c0 + c1·rx + c2·ry + c3·rz` with `c0 = d(X0,Y0,Z0)` and the
corner differences below, where `d(·)` are the cell's corner samples per output channel.
The branches are tested with `>=` **in exactly this order** (ties are order-dependent):

| # | condition | `c1` | `c2` | `c3` |
|---|-----------|------|------|------|
| 1 | `rx ≥ ry && ry ≥ rz` | `d(X1,Y0,Z0) − c0` | `d(X1,Y1,Z0) − d(X1,Y0,Z0)` | `d(X1,Y1,Z1) − d(X1,Y1,Z0)` |
| 2 | `rx ≥ rz && rz ≥ ry` | `d(X1,Y0,Z0) − c0` | `d(X1,Y1,Z1) − d(X1,Y0,Z1)` | `d(X1,Y0,Z1) − d(X1,Y0,Z0)` |
| 3 | `rz ≥ rx && rx ≥ ry` | `d(X1,Y0,Z1) − d(X0,Y0,Z1)` | `d(X1,Y1,Z1) − d(X1,Y0,Z1)` | `d(X0,Y0,Z1) − c0` |
| 4 | `ry ≥ rx && rx ≥ rz` | `d(X1,Y1,Z0) − d(X0,Y1,Z0)` | `d(X0,Y1,Z0) − c0` | `d(X1,Y1,Z1) − d(X1,Y1,Z0)` |
| 5 | `ry ≥ rz && rz ≥ rx` | `d(X1,Y1,Z1) − d(X0,Y1,Z1)` | `d(X0,Y1,Z0) − c0` | `d(X0,Y1,Z1) − d(X0,Y1,Z0)` |
| 6 | `rz ≥ ry && ry ≥ rx` | `d(X1,Y1,Z1) − d(X0,Y1,Z1)` | `d(X0,Y1,Z1) − d(X0,Y0,Z1)` | `d(X0,Y0,Z1) − c0` |

lcms2 closes the cascade with an unreachable `c1 = c2 = c3 = 0` fallback (only NaN fractions
could reach it, and `fclamp` removes NaN first); since the six orderings are exhaustive for
finite fractions, branch 6 is the `else` arm in the Rust transcription.

**Dimension selection** (`cmsintrp.c:1178-1310`). Tetrahedral serves exactly 3 inputs unless
the `CMS_LERP_FLAGS_TRILINEAR` hint is set — which lcms2 sets only for **Lab-indexed** CLUTs
(`ChangeInterpolationToTrilinear`, `src/cmsio1.c:516-533`, applied to B2A/devicelink pipelines
whose PCS is Lab). Trilinear/bilinear LERP order is X, then Y, then Z
(`TrilinearInterpFloat`, `cmsintrp.c:470-540`). Four inputs and above
(`Eval4InputsFloat`…`Eval15InputsFloat`, `cmsintrp.c:1038-1174`) slice the outermost axis:
floor + fraction on input 0, evaluate the two inner (N−1)-D sub-grids, blend linearly —
bottoming out in the 3-D tetrahedral base.
