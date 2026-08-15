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
| `itu-r-bt2408-8.pdf`     | Report ITU-R BT.2408-8 (2024) — HDR Reference White (203 cd/m²) for HDR production |
| `adobe-rgb-1998.pdf`     | Adobe RGB (1998) Color Image Encoding — primaries & γ (authentic Adobe file via the Internet Archive; Adobe's host no longer serves it) |
| `romm-rgb.pdf`           | ROMM RGB / ProPhoto white paper — the free primary reference for ISO 22028-2's reference encoding |

## Not vendored (paywalled — values transcribed inline below)

- **SMPTE ST 2084:2014** (PQ EOTF) and **SMPTE EG 432-1** (DCI-P3 with D65) — SMPTE, paywalled.
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

## PQ — SMPTE ST 2084 / ITU-R BT.2100 EOTF

Source: SMPTE ST 2084:2014; ITU-R BT.2100-2. Constants are exact dyadic rationals:

| const | value            | rational            |
|-------|------------------|---------------------|
| m1    | `0.1593017578125`| `(2610/4096) / 4`   |
| m2    | `78.84375`       | `(2523/4096) · 128` |
| c1    | `0.8359375`      | `3424/4096`         |
| c2    | `18.8515625`     | `(2413/4096) · 32`  |
| c3    | `18.6875`        | `(2392/4096) · 32`  |

Peak luminance `10000` cd/m². EOTF (E' → linear, normalized to [0,1]·10000 nits — signal → light
is the EOTF per ST 2084; the encode direction is the *inverse* EOTF):
`Y = ((max(E'^(1/m2) − c1, 0)) / (c2 − c3·E'^(1/m2)))^(1/m1)`.
Note `c1 = c3 − c2 + 1`.

---

## HDR / SDR reference white — ITU-R BT.2408

Source: Report ITU-R BT.2408-8 (2024) (`itu-r-bt2408-8.pdf`, verified 2026-06). **HDR Reference
White = 203 cd/m²** — the nominal signal level of graphics / diffuse white in PQ and HLG
production. (Earlier code mislabeled this "SDR reference white"; 203 is the *HDR* reference level.)
The classic **SDR reference white is 100 cd/m²**; BT.2408 is the framework that maps SDR 100 %
diffuse white onto HDR Reference White. PQ peak luminance is **10 000 cd/m²** (ST 2084, above).

These three levels are defined once in `gamut_core::luminance` (`HDR_REFERENCE_WHITE_NITS = 203`,
`SDR_REFERENCE_WHITE_NITS = 100`, `PQ_PEAK_NITS = 10000`) and shared by `gamut-color` (the BT.2020
PQ→SDR path tone-maps relative to 203) and `gamut-tonemap` (`DEFAULT_REINHARD_WHITE = 203/100 =
2.03`). The tone-curve operators are documented in `references/tonemap/README.md`.

---

## sRGB transfer — IEC 61966-2-1

EOTF (gamma→linear): `x ≤ 0.04045 ? x/12.92 : ((x+0.055)/1.055)^2.4`.
OETF (linear→gamma): `x ≤ 0.0031308 ? 12.92·x : 1.055·x^(1/2.4) − 0.055`.

---

## Linear transfer — ITU-T H.273 / ISO/IEC 23091-2 code point 8

`TransferCharacteristics = 8` is **Linear**, defined as the identity `V = Lc` over the
reference domain (H.273 Table 3, row 8). There is no constant to transcribe: the curve is
`f(x) = x`, implemented as `gamut_color::transfer::linear_eotf` and paired with BT.709
primaries (code point 1) as `SourceProfile::LINEAR_SRGB` — the scene-linear working space a
RAW pipeline demosaics, white-balances, and colour-matrixes in before applying its output
transfer.

H.273 is freely published by the ITU: <https://www.itu.int/rec/T-REC-H.273>.

Vendored in this directory as: `T-REC-H.273-202407-I!!PDF-E.pdf`.s

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

---

## XYB (JPEG XL opsin colour space) — ISO/IEC 18181-1 / libjxl

Sources: the pre-ISO Committee Draft `references/jxl/1908.03565.pdf` (defines the XYB space and
the normative inverse), and the **frozen** reference-implementation constants of **libjxl
0.12.0** — the exact version this workspace pins as its JXL oracle (`references/jxl/README.md`) —
vendored verbatim (BSD-3-Clause, header retained) as `references/jxl/opsin_params.h`.
Implemented by `gamut-color`'s `xyb` module; consumed by `gamut-jpeg`'s XYB colour mode and its
embedded ICC profile (regenerated and byte-pinned by `crates/gamut/tests/xyb_icc.rs`).

Forward: linear sRGB → opsin absorbance matrix → per-channel `∛(x + b) − ∛b` →
`X = (L′−M′)/2, Y = (L′+M′)/2, B = S′`.

Opsin absorbance matrix (`kOpsinAbsorbanceMatrix`; middle/last entries of each row are defined
as `1 −` the others, so each row sums to 1):
```
0.30                  1 − 0.078 − 0.30      0.078
0.23                  1 − 0.078 − 0.23      0.078
0.24342268924547819   0.20476744424496821   1 − kM20 − kM21
```
Bias (`kOpsinAbsorbanceBias`, all channels): `b = 0.0037930732552754493`.

Frozen inverse (`kDefaultInverseOpsinAbsorbanceMatrix` — transcribed, not re-derived: the decode
direction is normative and carries its own f32-rounded literals):
```
 11.031566901960783  −9.866943921568629   −0.16462299647058826
 −3.254147380392157   4.418770392156863   −0.16462299647058826
 −3.6588512862745097  2.7129230470588235   1.9459282392156863
```

Scaled-XYB byte encoding (`kScaledXYBOffset` / `kScaledXYBScale`; the **third stored channel is
`B − Y`**): `sᵢ = clamp((storedᵢ + offsetᵢ)·scaleᵢ, 0, 1)` with
```
offset = (0.015386134, 0.0, 0.27770459)
scale  = (22.995788804, 1.183000077, 1.502141333)
```

XYB ICC `A2B0` matrix (libjxl `jxl_cms_internal.h`, `CreateICCLutAtoBTagForXYB`): the literal
`0.5 · XYZ(D50)←linear-sRGB · inverse-opsin` (the 0.5 bakes in the mAB PCS-XYZ encoding
ceiling), verified against a fresh derivation in `crates/gamut/tests/xyb_icc.rs` before use:
```
 1.5170095  −1.1065225   0.071623
−0.050022    0.5683655  −0.018344
−1.387676    1.1145555   0.6857255
```

---

## YCbCr matrix coefficients — ITU-T H.273 §8.3 / ISO/IEC 23091-2

Source: ITU-T H.273 (2024-07) Table 4 (`MatrixCoefficients`) and §8.3, the non-constant-luminance
YCbCr ↔ RGB relations. Backs `gamut_color::YcbcrMatrix`.

The published luma weights are exact four-decimal values, so the derivation is exact integer
arithmetic — unlike the rest of this directory, that path is bit-exact and deterministic rather
than Tier-1 `f64`.

| Code point | Name | `Kr` | `Kb` |
| --- | --- | --- | --- |
| 1 | BT.709 / BT.1361 / sRGB-matrix | 0.2126 | 0.0722 |
| 5 | BT.470 System B,G / BT.601 625 | 0.2990 | 0.1140 |
| 6 | BT.601 525 / SMPTE 170M | 0.2990 | 0.1140 |
| 9 | BT.2020 non-constant luminance | 0.2627 | 0.0593 |

Code points 5 and 6 are distinct points naming identical coefficients; both are modeled so a `colr`
box read as 5 is written back as 5.

With `Kg = 1 − Kr − Kb`, the inverse (de-matrixing) is

```
R' = Y'                              + 2(1 − Kr)·Cr
G' = Y' − (2·Kb(1 − Kb)/Kg)·Cb − (2·Kr(1 − Kr)/Kg)·Cr
B' = Y' + 2(1 − Kb)·Cb
```

Range normalization at bit depth `bd`, with `max = 2^bd − 1` (H.273 §8.3):

| | luma offset | luma scale | chroma offset | chroma scale |
| --- | --- | --- | --- | --- |
| Limited ("studio swing") | `16 << (bd − 8)` | `219 << (bd − 8)` | `128 << (bd − 8)` | `224 << (bd − 8)` |
| Full | `0` | `max` | `2^(bd − 1)` | `max` |

`Y' = (Y − luma_offset)/luma_scale`, `Cb = (cb − chroma_offset)/chroma_scale`, and the output
sample is `round(max · R')` saturated to `0..=max` (the AV1 `Clip1` of `clip_pixel`).

Two consequences worth recording, both asserted by tests: the full-range chroma coefficients are
bit-depth independent (`max / chroma_scale = 1`), and the limited-range luma gain depends only on
`(range, bit_depth)`, never on the matrix.

Note that `gamut_color::ycbcr_to_rgb` does **not** use these relations: it is a bit-exact port of
libwebp's `VP8YUVToR/G/B`, kept so WebP decode matches libwebp per pixel. The two are both correct
BT.601 and differ by at most 1 LSB.
