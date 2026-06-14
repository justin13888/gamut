# gamut-icc

`gamut-icc` is a pure-Rust **ICC color profile** (ICC.1:2022) parser and serializer.

## Goals

Part of the [gamut](../../README.md) workspace, this crate models the ICC profile blob embedded in
images — the WebP `ICCP` chunk, the AVIF/HEIF `colr` box of type `prof`, a JPEG `APP2` segment — so
the format crates can read, preserve, and embed accurate color characterization. It is:

- **Memory-safe on hostile input.** `#![forbid(unsafe_code)]` — profiles are offset-indexed blobs
  from untrusted files.
- **Clean-slate from the spec.** Implemented from **ICC.1:2022** (profile v4.4, equivalent to
  ISO 15076-1; [`../../references/icc`](../../references/icc)), with v2 read support since most
  embedded profiles are still v2.
- **Dependency-light.** An ICC profile needs neither IFD nor XML machinery, so this crate builds
  only on [`gamut-core`](../gamut-core) plus [`md-5`](https://crates.io/crates/md-5) (the §7.2.18
  profile-ID digest) — distinct from CICP color signaling, which lives in
  [`gamut-color`](../gamut-color).

## Usage

```rust,no_run
use gamut_icc::{IccProfile, KnownTag, TagData};

# fn demo(bytes: &[u8]) -> Result<(), gamut_core::Error> {
let profile = IccProfile::parse(bytes)?;
if let Some(TagData::Xyz(white)) = profile.get(KnownTag::MediaWhitePoint.signature()) {
    println!("media white point: {:?}", white[0].to_f64());
}
let serialized = profile.to_bytes(); // spec-valid bytes, ready to re-embed
# Ok(()) }
```

The load-bearing element types decode semantically (`XYZType`, the curve types, the text types, the
`lut8`/`lut16`/`lutAToB`/`lutBToA` transforms, `namedColor2Type`, …); any other type is preserved
verbatim as `TagData::Raw`, so every profile round-trips losslessly. `IccReader`/`IccWriter` carry
options (strict parsing; profile-ID recomputation).

**Out of scope:** applying a profile's transform (a CMM) and constructing transforms from
`gamut-color` — the `to_f64`/`eval` accessors are the integration seam.

## Conformance

Differential-tested against **Little-CMS (lcms2)**: gamut-icc decodes lcms-synthesized profiles to
the same values lcms reports, and lcms re-opens gamut-icc's serialization as an equivalent profile.
See [STATUS.md](STATUS.md).

## License

Licensed under either of MIT or Apache-2.0 at your option.
