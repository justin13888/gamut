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

Differential oracle against **Little-CMS (lcms2)** (C FFI) for parse + re-serialize equivalence;
see [`gamut-icc/STATUS.md`](../../crates/gamut-icc/STATUS.md).
