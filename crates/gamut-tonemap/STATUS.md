# gamut-tonemap — implementation status

Tone-mapping math primitives: a `ToneCurve` trait plus built-in operators, implemented clean-slate
from the primary sources transcribed in [`references/tonemap`](../../references/tonemap/README.md).
Operators take non-negative **linear-light** input and are pure `f32 -> f32` maps; the absolute
reference luminances they normalize against live in `gamut_core::luminance`.

**Status:** ✅ implemented · ☐ planned.

## Operators

| Operator | Source | Status |
|----------|--------|--------|
| `ToneCurve` trait + blanket `Fn(f32) -> f32` impl | — | ✅ |
| `Linear` — identity passthrough | — | ✅ |
| `Clamp` — hard clamp to `[0, max]` | — | ✅ |
| `Exposure` — linear pre-scale (gain or photographic stops) | photographic convention | ✅ |
| `Reinhard` — `L / (1 + L)` | Reinhard et al. 2002, Eq. 3 | ✅ |
| `ReinhardExtended` — white-point variant | Reinhard et al. 2002, Eq. 4 | ✅ |
| `Aces` — filmic approximation | Narkowicz 2016 (fit to ACES RRT+ODT) | ✅ |
| `Hable` — Uncharted 2 filmic | Hable 2010 | ✅ |
| `Drago` — adaptive logarithmic | Drago et al. 2003, Eq. 4 | ✅ |

## Deferred

| Item | Notes | Status |
|------|-------|--------|
| Turnkey HDR→SDR helper | Pair a curve with `gamut-color` transfer functions (linearize → map → re-encode) behind an optional feature; needs a `gamut-color` runtime dependency. | ☐ |
| Curve-composition combinators | e.g. `curve.then(other)`; today compose with a closure: `\|x\| b.map(a.map(x))`. | ☐ |
| Other filmic fits | The Narkowicz ACES fit ships; the Stephen Hill fit needs RRT/ODT matrices (colour-coupled). | ☐ |

## Out of scope

`gamut-tonemap` is deliberately a scalar tone-curve library. The following are **not** provided here
— they belong to the surrounding pipeline (`gamut-color`, the codec crates):

- Colour-space conversion, gamut mapping, white-point adaptation.
- Transfer-function linearization / re-encoding (EOTF/OETF) — see `gamut-color`.
- Pixel I/O and alpha handling: apply a curve to RGB channels while preserving alpha in the caller.
- The full ACES RRT+ODT transform (colour-space-coupled).
