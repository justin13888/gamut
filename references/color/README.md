# Color-science constants — source of truth (issue #37)

This directory records the authoritative values for the color/tone-mapping math added
to `gamut-color` and `gamut-dsp` under issue #37, together with their primary-source
citations. Every constant baked into the code is listed here so the literals can be
audited against the standard rather than trusted by copy-paste.

The same-author upstream **chromahash** (`MIT OR Apache-2.0`) is the immediate provenance
for the *encoder-exact simplifications* (e.g. Adobe = pure `x^2.2`, ProPhoto = pure
`x^1.8`, BT.2020 = PQ→Reinhard@203). chromahash's `spec/validate.py` already derives the
OKLab `M1` matrices from first principles and checks them to `1e-7`; gamut reproduces that
derivation as a unit test (`gamut-color::matrix`) so the literal `M1_*` tables are
*computed-equivalent*, not merely transcribed.

> Determinism note: gamut implements these at **Tier-1 (correctness only)** using `std`
> f64 (`powf`/`cbrt`/`ln`). chromahash's bit-reproducible substrate (`cbrt_halley`,
> `portable_pow/exp/ln/cos`) is intentionally **not** ported — see issue #37. Therefore
> gamut's outputs agree with chromahash's golden vectors only within a small tolerance,
> not bit-for-bit.

---

## Vendored primary sources

The freely-published primary sources for the values below are vendored alongside this README so the
literals can be audited offline. The paywalled SMPTE/IEC/ISO editions cannot be redistributed — every
constant they define is instead transcribed with its exact value in the tables below (see
*Not vendored*).

| file | source |
|------|--------|
| `oklab-ottosson.html`    | Björn Ottosson, "A perceptual color space for image processing" — <https://bottosson.github.io/posts/oklab/> |
| `bradford-lindbloom.html`| Bruce Lindbloom, chromatic adaptation (Bradford CAT) — <http://www.brucelindbloom.com/Eqn_ChromAdapt.html> |
| `itu-r-bt709-6.pdf`      | ITU-R BT.709-6 — sRGB/BT.709 primaries & white point |
| `itu-r-bt2020-2.pdf`     | ITU-R BT.2020-2 — BT.2020 primaries & white point |
| `itu-r-bt2100-2.pdf`     | ITU-R BT.2100-2 — PQ/HLG systems (BT.2100-3 is now in force; `-2` is the edition cited below) |
| `adobe-rgb-1998.pdf`     | Adobe RGB (1998) Color Image Encoding — primaries & γ (authentic Adobe file via the Internet Archive; Adobe's host no longer serves it) |
| `romm-rgb.pdf`           | ROMM RGB / ProPhoto white paper — the free primary reference for ISO 22028-2's reference encoding |

## Not vendored (paywalled — values transcribed inline below)

- **SMPTE ST 2084:2014** (PQ inverse EOTF) and **SMPTE EG 432-1** (DCI-P3 with D65) — SMPTE, paywalled.
- **IEC 61966-2-1** (sRGB transfer functions) — IEC, paywalled.
- **ISO 22028-2** (ROMM/ProPhoto reference colour encoding) — ISO, paywalled; the ROMM RGB white paper
  above is the freely-published primary reference for the same primaries and encoding.

---

## OKLab matrices — Björn Ottosson

Source: "A perceptual color space for image processing", Björn Ottosson,
<https://bottosson.github.io/posts/oklab/> (verified 2026-06). Public-domain / MIT.
Nonlinearity between `M1` and `M2` is the **cube root** (γ = 1/3).

`M1` — linear sRGB → LMS cone response:
```
0.4122214708  0.5363325363  0.0514459929
0.2119034982  0.6806995451  0.1073969566
0.0883024619  0.2817188376  0.6299787005
```

`M2` — cube-root LMS (l'm's') → OKLab [L, a, b]:
```
0.2104542553  0.7936177850  -0.0040720468
1.9779984951 -2.4285922050   0.4505937099
0.0259040371  0.7827717662  -0.8086757660
```

`M2⁻¹` — OKLab → cube-root LMS (first column is all 1.0):
```
1.0  0.3963377774  0.2158037573
1.0 -0.1055613458 -0.0638541728
1.0 -0.0894841775 -1.2914855480
```

`M1⁻¹` — LMS → linear sRGB:
```
 4.0767416621 -3.3077115913  0.2309699292
-1.2684380046  2.6097574011 -0.3413193965
-0.0041960863 -0.7034186147  1.7076147010
```

Per-gamut `M1[gamut] = M_LMS · M_XYZ[gamut]` (with Bradford D50→D65 baked into ProPhoto)
are listed in `gamut-color/src/oklab.rs`; they are verified against the derivation below.

---

## PQ — SMPTE ST 2084 / ITU-R BT.2100 inverse EOTF

Source: SMPTE ST 2084:2014; ITU-R BT.2100-2. Constants are exact dyadic rationals:

| const | value            | rational            |
|-------|------------------|---------------------|
| m1    | `0.1593017578125`| `(2610/4096) / 4`   |
| m2    | `78.84375`       | `(2523/4096) · 128` |
| c1    | `0.8359375`      | `3424/4096`         |
| c2    | `18.8515625`     | `(2413/4096) · 32`  |
| c3    | `18.6875`        | `(2392/4096) · 32`  |

Peak luminance `10000` cd/m². Inverse EOTF (E' → linear, normalized to [0,1]·10000 nits):
`Y = ((max(E'^(1/m2) − c1, 0)) / (c2 − c3·E'^(1/m2)))^(1/m1)`.
Note `c1 = c3 − c2 + 1`.

---

## sRGB transfer — IEC 61966-2-1

EOTF (gamma→linear): `x ≤ 0.04045 ? x/12.92 : ((x+0.055)/1.055)^2.4`.
OETF (linear→gamma): `x ≤ 0.0031308 ? 12.92·x : 1.055·x^(1/2.4) − 0.055`.

---

## Encoder-exact transfer simplifications (chromahash)

These deliberately differ from the textbook curves; gamut exposes both so the bitstream a
metrics tool predicts matches what the encoder did:

| gamut       | encoder-exact (chromahash) | textbook                                   |
|-------------|----------------------------|--------------------------------------------|
| Adobe RGB   | `x^2.2`                    | `x^(563/256)` = `x^2.19921875`             |
| ProPhoto    | `x^1.8` (no toe)           | linear toe `slope 16` below `Eₜ = 1/512`   |
| BT.2020     | PQ→nits→Reinhard@203 nits  | pure ST 2084 → nits (no tone map)          |

---

## RGB primaries + white points (CIE 1931 xy)

Sources: ITU-R BT.709-6 (sRGB), ITU-R BT.2020-2 (BT.2020), SMPTE EG 432-1 (DCI-P3 with
D65), Adobe RGB (1998) Color Image Encoding, ISO 22028-2 / ROMM RGB (ProPhoto).

| gamut        | R              | G                | B                  | white |
|--------------|----------------|------------------|--------------------|-------|
| sRGB/BT.709  | (0.6400,0.3300)| (0.3000,0.6000)  | (0.1500,0.0600)    | D65   |
| Display P3   | (0.6800,0.3200)| (0.2650,0.6900)  | (0.1500,0.0600)    | D65   |
| Adobe RGB    | (0.6400,0.3300)| (0.2100,0.7100)  | (0.1500,0.0600)    | D65   |
| BT.2020      | (0.7080,0.2920)| (0.1700,0.7970)  | (0.1310,0.0460)    | D65   |
| ProPhoto RGB | (0.734699,0.265301)|(0.159597,0.840403)|(0.036598,0.000105)| D50  |

White points: **D65** = (0.3127, 0.3290); **D50** = (0.3457, 0.3585).

---

## Bradford chromatic adaptation (cone response matrix)

Source: Lindbloom, <http://www.brucelindbloom.com/Eqn_ChromAdapt.html>; CIECAM/ICC Bradford
CAT (verified 2026-06).
```
 0.8951  0.2664 -0.1614
-0.7502  1.7135  0.0367
 0.0389 -0.0685  1.0296
```
Adaptation: `M_adapt = M_B⁻¹ · diag(cone_dst / cone_src) · M_B`, applied for non-D65
gamuts (ProPhoto's D50→D65) before the LMS projection.

---

## µ-law companding (chromahash v0.6)

`compress(v) = sign(v)·ln(1+µ|v|)/ln(1+µ)`, `expand(c) = sign(c)·((1+µ)^|c| − 1)/µ`.
Quantization uses an **odd** level count: `max_idx = 2^bits − 2` (the top code is never
written), so the center index dequantizes to exactly 0. Round-half-away-from-zero.
Defaults: `µ_L = 5.0`, `µ_C = 8.0`, `µ_alpha = 5.0`.
