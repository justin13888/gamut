# gamut-cmm — ICC colour management module status

**Epic: GitHub issue #323.** The colour management module (CMM) over the profiles
[`gamut-icc`](../gamut-icc) parses: builds runnable colour transforms (device→PCS→device) and
evaluates them over interleaved `f64` pixels. Data layouts follow **ICC.1:2022**
([`references/icc`](../../references/icc)); scope is the ICC v2/v4 still-image profile set.
Runtime dependencies: `gamut-icc`, `gamut-color`, `gamut-core`; `#![forbid(unsafe_code)]`.

**Keystone:** the **pipeline/stage model** — a colour transform as a validated chain of `Stage`s.
`Pipeline::new` is the validity boundary: every channel count (declared ends, per-stage
input/output, every adjacent seam) is checked exactly once at construction, so a constructed
pipeline always evaluates, allocation-free, by ping-ponging two `[f64; MAX_CHANNELS]` stack
buffers. Every later phase is an additive `Stage` variant plus its `eval` arm (the match is
deliberately exhaustive so the compiler forces both to land together) or a builder that emits
pipelines.

**Oracle:** **Little-CMS (lcms2)** via the dev-only FFI oracle `tooling/lcms2-oracle`
([`references/cmm`](../../references/cmm/README.md)). ICC.1 specifies data layouts, not CMM
behaviour, so where the spec is silent (interpolation, clamping — including `Clamp`'s
NaN → 0.0 choice) observable semantics follow lcms2; differential tests arrive with the phases
that add behaviour (#325 onward).

## Phases

| Phase | Issue | Scope | Status |
| ----- | ----- | ----- | ------ |
| P1 | #324 | Scaffold + keystone: `Pipeline`/`Stage` model, `Transform` entry trait, `CmmError`, workspace wiring | ✅ |
| P2 | #325 | Curve stages: `ToneCurve` (`curveType`/`parametricCurveType` evaluation, monotonicity detection, analytic + lcms2-shaped numeric inversion) + `Stage::Curves` | ✅ |
| P3 | #326 | CLUT stage: multi-dimensional interpolation (lcms2-matching) — `ClutTable`/`ClutInterpolation` + `Stage::Clut` | ✅ |
| P4 | #327 | Profile linking: matrix/TRC (shaper) profile pairs | ☐ |
| P5 | #328 | Profile linking: LUT (`lut8`/`lut16`/`mAB `/`mBA `) profile pairs | ☐ |
| P6 | #329 | Rendering intents + black-point compensation | ☐ |
| P7 | #330 | Transform chaining + typed pixel buffers | ☐ |

## Settled decisions (P2, tone curves)

- **Endpoint semantics:** `ToneCurve::eval` clamps domain **and** range to `[0, 1]` in every
  representation, forward and inverse — the convention of `gamut_icc::Curve::eval`, extended to
  parametric curves whose raw closed forms can leave the range.
- **Unknown parametric types:** `gamut_icc::ParametricCurve::eval` silently evaluates a
  `function_type > 4` as the identity (unreachable from parsed profiles, reachable from
  hand-built values); `ToneCurve::new` guards the trap with the typed
  `CmmError::UnsupportedParametricType`.
- **Inversion:** analytic closed forms (lcms2's negated-type formulas, at full `f64` precision —
  a gamma inverse is *not* re-encoded through `u8Fixed8`) for identity, pure gamma, and
  parametric types 1–4 with `g > 0`, `a > 0` (types 3–4 also `c > 0`, `d ∈ [0, 1]`); everything
  else — sampled tables, degenerate-but-monotonic parameterizations — reverses numerically into
  a 4096-entry table shaped after `cmsReverseToneCurveEx` (same entry count, interval-scan
  directions, and flat-run convention as the oracle). Non-monotonic and constant curves are
  rejected with `CmmError::NonMonotonicCurve`.
- **Flat segments:** a flat run's value maps to the run edge adjoining the curve's larger values
  (lcms2's `y2`-for-ascending / `y1`-for-descending choice). One deliberate deviation: a
  reversal target below a *descending* table's minimum maps to the correct domain end `1`,
  where lcms2's carried-coefficient quirk emits `0`; range-spanning tables (every table the
  differential tests share with the oracle) never hit the case.

## Settled decisions (P3, CLUT interpolation)

- **Interpolation modes:** `ClutInterpolation::Tetrahedral` (default for ≥ 3 input channels,
  lcms2's device-CLUT selection) is the exact six-branch `TetrahedralInterpFloat`
  decomposition — `>=` cascade in lcms2's order, so ties resolve identically (transcribed in
  [`references/cmm`](../../references/cmm/README.md)) — with ≥ 4 inputs recursing lcms2's
  `Eval4InputsFloat`… scheme (outermost-axis slice, two inner evaluations, linear blend) down
  to the 3-D tetrahedral base. `ClutInterpolation::Multilinear` (default and only mode for
  1–2 inputs, where the forms coincide) is classic 2ᴺ-corner multilinear at every dimension;
  requesting `Tetrahedral` below 3 inputs is a typed `ClutGeometry` error, not a silent
  fallback. The mode is carried per table (`ClutTable::with_interpolation`) because lcms2
  forces trilinear for **Lab-indexed** CLUTs at profile-read time
  (`ChangeInterpolationToTrilinear`) — #328's linking layer selects it there.
- **Input mapping:** lcms2's `fclamp` (NaN and everything below `1e-9` → `0.0`, above `1.0` →
  `1.0`), `px = fclamp(v)·(n−1)`, floor cell + fraction, and the exact-`1.0` rule (upper node
  = lower node when the clamped input is `≥ 1.0`).
- **Single-node axes (deliberate divergence):** an axis with one grid node interpolates as
  constant. lcms2's 2-D+ float routines have no `Domain == 0` guard and read one node past
  the end there; this crate pins the in-bounds semantics (the differential tests avoid 1-node
  axes).
- **Normalization:** samples normalize once at construction by the CLUT's precision full
  scale (255 for 8-bit data widened to `u16` at parse, 65535 for 16-bit) — never a blanket
  65535.
- **Bounds:** CLUT input dimensions cap at 15 (lcms2 `MAX_INPUT_DIMENSIONS`; ICC device
  spaces stop at 15 colorants), outputs at the pipeline-wide 16.

## Deferred / out of scope

| Item | Notes | Status |
|------|-------|--------|
| iccMAX (`ICC.2:2019`) | A separate, parallel next-generation format (spectral PCS, v5 header); not an extension of ICC.1 and unimplementable against the lcms2 oracle. See [`references/icc`](../../references/icc/README.md). | ✗ out of scope |
| `multiProcessElementsType` (`mpet`) + `DToBx`/`BToDx` tags | The v4/iccMAX general-purpose processing pipeline; `gamut-icc` preserves it as `Raw`, and this CMM does not evaluate it. | ✗ out of scope |
| Integer/`f32` fast paths | Evaluation is `f64` throughout at Tier-1 (correctness only, not bit-reproducible — the `gamut-color` posture, see [`references/color`](../../references/color/README.md)). | ☐ unplanned |

## Validation

Inline unit tests (stage evaluation against hand-computed exact-dyadic values, clamp semantics
incl. NaN, object safety; tone-curve internals — reversal scan directions, flat-run edges,
out-of-range clamps, closed-form inverses — pinned against hand-derived exact values; CLUT
internals — node addressing/interleaving, per-branch tetrahedral probes and exact-tie branch
ordering against hand-transcribed formulas, multilinear vs an independent naive corner-weight
implementation up to 4-D, the measured tetrahedral-vs-trilinear divergence bound, `fclamp` and
single-node-axis edges, 15-D acceptance and geometry rejection) plus the `tests/pipeline.rs`
integration suite (construction-time rejection with exact typed variants and fields, boundary
channel counts, empty-pipeline identity, multi-pixel `Transform` buffer contract, composition,
per-channel `Stage::Curves` evaluation, curves → CLUT → matrix hand-checked pixels) and the
differential suites against lcms2: `tests/oracle_curves.rs` (forward sweeps for
identity/gamma/sampled/all five parametric types, inversion vs `cmsReverseToneCurveEx`,
analytic-vs-numeric inverse agreement, round-trip batteries over gammas, parametric curves, and
seeded random tables with and without flat runs) and `tests/oracle_clut.rs` (float-pipeline
sweeps against lcms2's `TetrahedralInterpFloat`/`Eval4InputsFloat`/1-D/2-D interpolators to
f32-rounding tightness, plus end-to-end `cmsDoTransform` sweeps over synthesized devicelink
CLUT probe profiles to 16-bit-quantization tightness). Gates: `mise run test` / `lint` /
`fmt-check` / `coverage` (≥ 80%) / `mise run mutants-crate gamut-cmm`.
