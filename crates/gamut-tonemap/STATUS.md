# gamut-tonemap — tone-mapping primitives status

**v1 stabilization: GitHub issue #188.** Tone-mapping math primitives: the `ToneCurve` trait plus
eight built-in operators, implemented clean-slate from the primary sources transcribed in
[`references/tonemap`](../../references/tonemap/README.md). Operators take non-negative
**linear-light** input and are pure `f32 -> f32` maps; the absolute reference luminances they
normalize against live in `gamut_core::luminance`. Runtime dependency: `gamut-core` only;
`#![forbid(unsafe_code)]`.

**Keystone:** the **construction-time validity boundary**. Parameterized operators validate their
parameters — including the derived divisors/normalizers the hot loop actually uses (`white²`,
`partial(white)`, Drago's prefactor) — exactly once, at construction, so `map` stays branch-lean
and the numeric contract (never NaN, non-negative, monotone up to f32 rounding) holds for every
constructible operator across the full f32 input range.

## Public surface (frozen at v1)

| Item | Shape | Openness |
| ---- | ----- | -------- |
| `ToneCurve` | `map(&self, f32) -> f32` + provided `map_slice(&mut [f32])`; dyn-compatible | open trait; blanket impl covers every `Fn(f32) -> f32` |
| `Linear`, `Reinhard`, `Aces` | parameterless operators (zero-sized) | literal-constructible by design — never `#[non_exhaustive]` |
| `Clamp`, `Exposure`, `ReinhardExtended`, `Hable`, `Drago` | validated constructors returning `gamut_core::Result`; by-value accessors | private fields — adding fields is non-breaking |
| `constants::*` | `SDR_WHITE_NORMALIZED`, `DEFAULT_REINHARD_WHITE`, `DEFAULT_HABLE_WHITE`, `DEFAULT_DRAGO_BIAS` | documented literals backing the `Default` impls |

All operators are `Debug + Clone + Copy + PartialEq` and re-exported at the crate root. Adding
operators, trait impls, or provided trait methods stays backward-compatible; removing or reshaping
any of the above would not.

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

## Settled design decisions (intentional, not gaps)

- **The blanket `Fn(f32) -> f32` impl trades away all forwarding impls.** Because `&F`, `&mut F`,
  `Box<F>`, and `Rc<F>` are themselves `Fn` whenever `F` is, coherence forecloses
  `impl ToneCurve for &T` (and `Box<T>`, `Rc<T>`, …) forever — in particular `Box<dyn ToneCurve>`
  will never implement `ToneCurve` itself. The blanket impl is its own escape hatch
  (`|x| boxed.map(x)`). Chosen deliberately: closures-as-curves is the ergonomic core of the API,
  every built-in is `Copy`, and `&dyn ToneCurve` works for dynamic dispatch.
- **f32 scalar math, Tier-1 correctness-only** (determinism note in
  `references/tonemap/README.md`). `gamut-color` keeps its own f64 encoder-exact
  `bt2020_pq_to_sdr`; the two implementations are proven to agree by a dev-dependency cross-check
  test rather than shared code, so tonemap never becomes a runtime dependency of the colour
  pipeline.
- **Numeric contract over literal transcription at f32 extremes.** Every operator is NaN-free
  across the entire f32 input range: `Aces` evaluates at `min(x, 8)` inside its exact saturation
  region, `Hable` clamps its argument where the curve sits within one ULP of its asymptote (both
  branchless), `ReinhardExtended` evaluates Eq (4) in factored form. Each deviation is derived and
  justified in `references/tonemap/README.md`; all published golden values are unchanged.
- **`Aces` names the Narkowicz fit.** The official colour-coupled ACES RRT+ODT — and the Hill
  fit — need 3×3 matrices and could never be a scalar `ToneCurve`, so there is no ambiguity to
  reserve the name against.
- **`Default` impls are the API; `constants` holds the documented literals** (mirroring the
  `gamut_core::luminance` precedent). No `Default` for `Exposure` or `Drago` — there is no
  canonical exposure, and a scene maximum cannot be guessed.
- **Composition stays a closure.** `|x| b.map(a.map(x))` composes curves today; a `then`
  combinator would be additive if ergonomics ever demand it.

## Deferred (all additive — none blocks v1)

| Item | Notes | Status |
|------|-------|--------|
| Turnkey HDR→SDR helper | Pair a curve with `gamut-color` transfer functions (linearize → map → re-encode) behind an optional feature; needs a `gamut-color` runtime dependency. | ☐ |
| Curve-composition combinators | e.g. `curve.then(other)` as a provided trait method; today compose with a closure: `\|x\| b.map(a.map(x))`. | ☐ |
| Other filmic fits | The Narkowicz ACES fit ships; the Stephen Hill fit needs RRT/ODT matrices (colour-coupled), so it would be a new, non-scalar API. | ☐ |

## Out of scope

`gamut-tonemap` is deliberately a scalar tone-curve library. The following are **not** provided here
— they belong to the surrounding pipeline (`gamut-color`, the codec crates):

- Colour-space conversion, gamut mapping, white-point adaptation.
- Transfer-function linearization / re-encoding (EOTF/OETF) — see `gamut-color`.
- Pixel I/O and alpha handling: apply a curve to RGB channels while preserving alpha in the caller.
- The full ACES RRT+ODT transform (colour-space-coupled).

## Validation

Inline unit tests: per-operator fixed points plus independent golden values transcribed from the
primary sources, monotonicity grids, constructor-rejection cases (including parameters whose
*derived* math degenerates), one adversarial f32-extremes sweep (never-NaN / non-negative at
`{0, 1e20, f32::MAX}` for all eight operators), and the Reinhard ↔ `gamut-color` cross-check.
Divan benches cover `map_slice` throughput for every operator (`cargo bench -p gamut-tonemap`).
Gates: `mise run test` / `lint` (`clippy -D warnings`, `missing_docs` fatal) / `fmt-check` /
`coverage` (≥ 80%) / `mise run mutants-crate gamut-tonemap`.
