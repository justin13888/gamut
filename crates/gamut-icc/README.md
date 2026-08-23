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
if let Some(TagData::Xyz(white)) = profile.get(KnownTag::MediaWhitePoint) {
    println!("media white point: {:?}", white[0].to_f64());
}
let serialized = profile.to_bytes()?; // spec-valid bytes, ready to re-embed
# Ok(()) }
```

**Every ICC.1:2022 §10 element type decodes semantically** — the `XYZType`, curve, and text types;
the `lut8`/`lut16`/`lutAToB`/`lutBToA` transforms; `namedColor2Type`; the measurement/signalling
types (`chromaticityType`, `cicpType`, `measurementType`, `viewingConditionsType`, `dataType`); the
colorant and generic array types; and the profile-sequence, response-curve, and dictionary types.
Only genuinely unmodelled types (e.g. iccMAX's `multiProcessElementsType`) fall back to
`TagData::Raw`, which round-trips byte-for-byte. `IccProfile::validate` additionally reports the §8
required tags a profile is missing for its device class, and `IccReader`/`IccWriter` carry options
(strict parsing; profile-ID recomputation). To build a profile from scratch,
`ProfileHeader::new(device_class, color_space)` supplies spec-valid header defaults; serialization
validates the model and rejects data that would produce a corrupt profile (a *parsed* profile
always re-serializes).

**Out of scope:** applying a profile's transform — that is [`gamut-cmm`](../gamut-cmm), the
workspace CMM (epic #323), for which the `to_f64`/`eval` accessors are the integration seam — and
**iccMAX** (`ICC.2`), a separate next-generation format (see
[`../../references/icc`](../../references/icc)).

## Inspecting real profiles

The workspace CLI extracts and inspects the profile embedded in a real photo:

```console
$ gamut icc DSC_0001.JPG        # camera JPEG (APP2), PNG (iCCP), TIFF/DNG (tag 34675),
$ gamut icc --verify-id sRGB.icc  # or a standalone .icc profile
```

## Conformance

Differential-tested against **Little-CMS (lcms2)**: gamut-icc decodes lcms-synthesized profiles to
the same values lcms reports (including the measurement/viewing/cicp tags and real device-link
`pseq`/`psid` sequences), and lcms re-opens gamut-icc's serialization as an equivalent profile.
See [STATUS.md](STATUS.md).

## License

Licensed under either of MIT or Apache-2.0 at your option.
