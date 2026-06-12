# ICC profiles (International Color Consortium)

Reference specifications for the `gamut-icc` crate.

## Authoritative editions

- **ICC.1:2022 (Profile version 4.4.0.0)** — the current ICC profile format specification, published
  freely by the International Color Consortium: <https://www.color.org/specification/ICC.1-2022-05.pdf>.
  Technically equivalent to **ISO 15076-1**. This is the edition `gamut-icc` targets.
- **ICC v2 (ICC.1:2001-04 and earlier)** — the still-ubiquitous legacy profile version; supported for
  reading, since the overwhelming majority of profiles embedded in real images are v2.

An ICC profile is a self-describing binary blob: a 128-byte header, a tag table, and tag element
data — independent of any IFD/XML structure, so `gamut-icc` depends only on `gamut-core`.

## Conformance

Differential oracle against **Little-CMS (lcms2)** (C FFI) for parse + re-serialize equivalence;
see [`gamut-icc/STATUS.md`](../../crates/gamut-icc/STATUS.md).
