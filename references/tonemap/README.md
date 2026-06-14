# Tone-mapping operators — source of truth (issue #188)

This directory records the authoritative definitions for every tone-mapping operator implemented by
`gamut-tonemap`, together with their primary-source citations. Each coefficient and formula baked
into the code is transcribed here so the literals can be audited against the source rather than
trusted by copy-paste. The built-in operators are implemented **clean-slate** from these published
definitions, not wrapped from a C library.

The reference white-luminance levels these operators normalize against (HDR 203 cd/m², SDR
100 cd/m², PQ peak 10 000 cd/m²) are defined once in `gamut_core::luminance` and documented under
`references/color/README.md` (ITU-R BT.2408, SMPTE ST 2084 / ITU-R BT.2100).

> Determinism note: `gamut-tonemap` evaluates these in **`f32`** scalar math (Tier-1, correctness
> only) using `std` `powf`/`ln`/`log10`. Golden values below are computed in higher precision; the
> crate's tests compare to them within a small `f32` tolerance.

---

## Vendored primary sources

Freely-published / openly-licensed primary sources are vendored alongside this README so the
literals can be audited offline.

| file | source |
|------|--------|
| `reinhard-2002-photographic-tone-reproduction.pdf` | Reinhard, Stark, Shirley, Ferwerda, "Photographic Tone Reproduction for Digital Images", SIGGRAPH 2002 — Univ. of Utah tech report UUCS-02-001 (freely published); DOI <https://doi.org/10.1145/566654.566575> |
| `drago-2003-adaptive-logarithmic-mapping.pdf` | Drago, Myszkowski, Annen, Chiba, "Adaptive Logarithmic Mapping For Displaying High Contrast Scenes", Eurographics 2003 / CGF 22(3) — © Eurographics; author copy via MPI Informatik (`resources.mpi-inf.mpg.de/tmo/logmap/`) |
| `aces-filmic-narkowicz.html` | Krzysztof Narkowicz, "ACES Filmic Tone Mapping Curve", 2016-01-06 — <https://knarkowicz.wordpress.com/2016/01/06/aces-filmic-tone-mapping-curve/> (author releases the code as **CC0 / MIT**) |
| `uncharted2-hable-filmicworlds.html` | John Hable, "Filmic Tonemapping Operators", filmicworlds.com, 2010-05-05 (GDC 2010 "Uncharted 2: HDR Lighting") — <http://filmicworlds.com/blog/filmic-tonemapping-operators/> |

Cross-referenced (not duplicated here): `references/color/itu-r-bt2408-8.pdf` (HDR Reference White
203 cd/m²) and the SMPTE ST 2084 PQ constants transcribed in `references/color/README.md`.

The official **ACES** system (the curve below only *approximates*) is the Academy Color Encoding
System, <https://github.com/ampas/aces-dev> — not vendored; the Narkowicz fit is the immediate
provenance of the coefficients.

---

## Reinhard — global photographic operators (2002)

Source: Reinhard et al. 2002, §3.1 "Initial luminance mapping" (verified 2026-06). Code:
`gamut_tonemap::operators::{Reinhard, ReinhardExtended}`.

**Simple operator, Eq (3):**

```
L_d = L / (1 + L)
```

Maps `[0, ∞) → [0, 1)`. High luminances are scaled by ≈`1/L`, low luminances by ≈`1`; the
denominator is a graceful blend, guaranteeing all luminances land in a displayable range.

**Extended operator (allows high luminances to burn out), Eq (4):**

```
L_d = L · (1 + L / L_white²) / (1 + L)
```

`L_white` is "the smallest luminance that will be mapped to pure white" (`map(L_white) = 1`). Setting
`L_white` to the scene maximum `L_max` or higher avoids burn-out; as `L_white → ∞` the operator
reverts to Eq (3).

`gamut-tonemap` default `DEFAULT_REINHARD_WHITE = 2.03` = BT.2408 HDR Reference White (203 cd/m²) /
SDR diffuse white (100 cd/m²). Golden: `Reinhard.map(1)=0.5`, `map(3)=0.75`;
`ReinhardExtended{4}.map(4)=1.0`, `map(2)=0.75`.

> Note: Reinhard's key-value pre-scale (Eq 1–2, `L = (a/L̄_w)·L_w`, an exposure/auto-exposure step)
> is **not** part of these operators — `gamut-tonemap` models it with the composable `Exposure`
> operator (below) so each curve stays a pure `f32 → f32` map.

---

## Exposure — pre-scaling

Source: standard photographic exposure convention (powers of two / "stops"; APEX system). Code:
`gamut_tonemap::operators::Exposure`.

```
map(x) = x · gain          gain = 2^stops      (one stop doubles exposure)
```

A composable linear gain applied before a curve. Models Reinhard's key scaling, the ACES "×0.6"
input pre-exposure, and Hable's "×2.0" exposure bias as an explicit operator rather than a hidden
constant. Golden: `Exposure::new(2).map(3)=6`; `Exposure::from_stops(1).scale()=2`.

---

## ACES filmic — Narkowicz approximation (2016)

Source: Narkowicz 2016 (`aces-filmic-narkowicz.html`, verified 2026-06), license CC0/MIT. Code:
`gamut_tonemap::operators::Aces`.

```
ACESFilm(x) = saturate( (x·(a·x + b)) / (x·(c·x + d) + e) )
a = 2.51   b = 0.03   c = 2.43   d = 0.59   e = 0.14
```

`saturate` clamps to `[0, 1]`. This is a **luminance-only rational fit** to `ODT(RRT(x))` sampled in
REC.709 / D65 (display gamma 2.4 removed); it is **not** the official ACES RRT+ODT transform (which
is colour-space-coupled — AP0/AP1 matrices, glow/red modifiers — and out of scope for a scalar
curve). The input is assumed pre-exposed (so `1 → ≈0.8`); apply `Exposure::new(0.6)` first to match
the original ACES curve. Golden: `map(0)=0`, `map(0.5)≈0.616307`, `map(1)≈0.803797`, `map(1e6)→1`
(saturates; the unclamped limit is `a/c ≈ 1.033`).

---

## Hable / Uncharted 2 filmic (2010)

Source: Hable 2010 (`uncharted2-hable-filmicworlds.html`, verified 2026-06). Code:
`gamut_tonemap::operators::Hable`.

```
partial(x) = ((x·(A·x + C·B) + D·E) / (x·(A·x + B) + D·F)) − E/F
A = 0.15 (shoulder strength)   B = 0.50 (linear strength)   C = 0.10 (linear angle)
D = 0.20 (toe strength)        E = 0.02 (toe numerator)     F = 0.30 (toe denominator)
W = 11.2 (linear white point)

map(x) = partial(x) / partial(W)
```

White-point normalized so `map(W) = 1` and `map(0) = 0`. The original presentation additionally
applied an exposure bias of 2.0 (`curr = partial(2·x)`) — modelled here by composing
`Exposure::new(2.0)` — and a final `pow(·, 1/2.2)` display re-encode, which belongs to the target
transfer function (`gamut-color`), not the tone curve. Default `W = DEFAULT_HABLE_WHITE = 11.2`.
Golden (`W = 11.2`): `map(0)=0`, `map(1)≈0.304300`, `map(4)≈0.713240`, `map(11.2)=1`.

> Hable later (2017, "Filmic Tonemapping with Piecewise Power Curves") proposed a different,
> piecewise operator; the curve above is the original 2010 Uncharted 2 operator.

---

## Drago — adaptive logarithmic mapping (2003)

Source: Drago et al. 2003, §3 (verified 2026-06). Code: `gamut_tonemap::operators::Drago`.

Bias power function, Eq (3) (Perlin & Hoffert):

```
bias_b(t) = t^(log(b) / log(0.5))
```

Main tone-mapping function, Eq (4):

```
L_d = (L_dmax · 0.01) / log10(L_wmax + 1)  ·  log(L_w + 1) / log( 2 + ((L_w / L_wmax)^(log(b)/log(0.5))) · 8 )
```

- `L_w` — world (scene) luminance; `L_wmax` — maximum scene luminance (`world_max`).
- `L_dmax` — maximum display luminance; the paper uses **100 cd/m²** (CRT reference), so
  `L_dmax · 0.01 = 1` and `L_d` is display-relative in `[0, 1]`.
- `b` — bias, useful range **[0.5, 1.0]**, default **0.85** (`DEFAULT_DRAGO_BIAS`). For `b < 0.7` the
  output can exceed 1.0 (the display clamps to `L_dmax`); `map` is a faithful, clamp-free
  transcription of Eq (4).
- The numerator/denominator log base cancels (Eq 2, change of base), so `ln` is used; only the
  `log10(L_wmax + 1)` normalization is base-specific.

Fixed points: `map(0) = 0` and `map(L_wmax) = 1`. Golden (`L_wmax = 100`, `b = 0.85`):
`map(10) ≈ 0.630858`.

---

## Cross-reference

- Luminance reference levels: `gamut_core::luminance`; provenance in `references/color/README.md`
  (ITU-R BT.2408 HDR Reference White; SMPTE ST 2084 PQ peak).
- The `Reinhard` operator is cross-checked at test time against `gamut-color`'s `bt2020_pq_to_sdr`
  (which applies the same `L/(1+L)` step) so the two crates' implementations are proven to agree.
