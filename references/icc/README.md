# ICC profiles (International Color Consortium)

Reference specifications for the `gamut-icc` crate.

## Authoritative editions (vendored)

- `icc.1-2022-05.pdf` — **ICC.1:2022 (Profile version 4.4.0.0)** — the current ICC profile format
  specification, technically equivalent to **ISO 15076-1**, and the edition `gamut-icc` targets.
  Published freely by the International Color Consortium:
  <https://www.color.org/specification/ICC.1-2022-05.pdf>.
- `icc.1-2001-04.pdf` — **ICC v2 (ICC.1:2001-04, a revision of ICC.1:1998-09)** — the still-ubiquitous
  legacy profile version; supported for reading, since the overwhelming majority of profiles embedded
  in real images are v2. Published freely by the ICC (`ICC_Minor_Revision_for_Web.pdf`):
  <https://www.color.org/icc_specs2.xalter>.

An ICC profile is a self-describing binary blob: a 128-byte header, a tag table, and tag element
data — independent of any IFD/XML structure, so `gamut-icc` depends only on `gamut-core`.

## Conformance

Differential oracle against **Little-CMS (lcms2)** (C FFI, `tooling/lcms2-oracle`) for parse +
re-serialize equivalence; see [`gamut-icc/STATUS.md`](../../crates/gamut-icc/STATUS.md).

## Formulas implemented by `gamut-icc`

The numeric encodings and closed forms the crate decodes/evaluates, with their spec sections. All
multi-byte values are big-endian.

### Fixed-point numbers (§4)

| Type | On disk | Value |
| ---- | ------- | ----- |
| `s15Fixed16Number` (§4.6) | signed `i32` | `raw / 65536` (e.g. `0x0001_0000` = 1.0, `0xFFFF_0000` = -1.0) |
| `u16Fixed16Number` (§4.7) | unsigned `u32` | `raw / 65536` |
| `u8Fixed8Number` (§4.5) | unsigned `u16` | `raw / 256` |

`XYZNumber` (§4.14) is three `s15Fixed16`; the header PCS illuminant is the D50 value
`(0.9642, 1.0, 0.8249) ≈ (0x0000_F6D6, 0x0001_0000, 0x0000_D32D)`. `dateTimeNumber` (§4.2) is six
`u16` (year, month, day, hour, minute, second, UTC).

### Profile ID — MD5 (§7.2.18)

The 16-byte profile ID is the MD5 of the *entire* profile with three header fields set to zero
first: **profile flags** (bytes 44–47), **rendering intent** (64–67), and **profile ID** itself
(84–99). The `size` field is left as written.

### Tone curves

`curveType` (§10.6) by entry count `n`: `n = 0` → identity `Y = X`; `n = 1` → gamma
`Y = X^g` with `g` a `u8Fixed8`; `n ≥ 2` → a uniform table over `[0, 1]`, samples `u16 / 65535`,
linearly interpolated.

`parametricCurveType` (§10.18), parameters in order `g, a, b, c, d, e, f`:

| Type | Function |
| ---- | -------- |
| 0 | `Y = X^g` |
| 1 | `Y = (aX + b)^g` for `X ≥ -b/a`, else `0` |
| 2 | `Y = (aX + b)^g + c` for `X ≥ -b/a`, else `c` |
| 3 | `Y = (aX + b)^g` for `X ≥ d`, else `cX` |
| 4 | `Y = (aX + b)^g + e` for `X ≥ d`, else `cX + f` |

### LUT transforms (§10.10–10.13)

`lut8Type`/`lut16Type` apply, in order: 3×3 matrix → input tables → CLUT → output tables. CLUT
samples normalize as `value / 255` (lut8) or `value / 65535` (lut16).

`lutAToBType` (`mAB `, device→PCS) applies A-curves → CLUT → M-curves → matrix(3×3 + 3 offset) →
B-curves; `lutBToAType` (`mBA `, PCS→device) reverses this: B-curves → matrix → M-curves → CLUT →
A-curves. Every stage but the B-curves is optional, signalled by a zero offset in the element
header; sub-elements are 4-byte aligned.
