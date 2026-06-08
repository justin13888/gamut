# gamut-isobmff

`gamut-isobmff` writes the ISO Base Media File Format (ISOBMFF) box structure that the AVIF — and
later HEIC — containers are built from.

## Goals

Part of the [gamut](../../README.md) workspace, this crate exists to:

- **Own the container, not the codec.** It serializes the ISOBMFF box tree (`ftyp`, `meta`, `mdat`,
  …) and leaves the coded bitstream to the codec crates, so AVIF and HEIC share one container
  implementation.
- **Emit the minimal valid box set.** M0 writes exactly the boxes a single still-image item needs
  (AVIF v1.2.0 §9.1.1): `ftyp`, then a `meta` box holding `hdlr`/`pitm`/`iloc`/`iinf`(`infe`)/
  `iprp`(`ipco`+`ipma`), followed by an `mdat` carrying the AV1 temporal unit — see
  [`write_avif_still`].
- **Stay spec-faithful and cross-checked.** Box byte layouts follow ISO/IEC 14496-12 (ISOBMFF) and
  ISO/IEC 23008-12 (HEIF) — paywalled and not in [`../../references/`](../../references) — verified
  against the public AVIF v1.2.0 box table and libavif/ffmpeg output, and validated with `avifdec`.
- **Stay memory-safe.** `#![forbid(unsafe_code)]`.

## Usage

```rust
use gamut_isobmff::{Av1cConfig, AvifStillImage, NclxColr, write_avif_still};

// `item_data` is the AV1 temporal unit produced by `gamut-av1`; `av1c`/`nclx` mirror the
// AV1 sequence header. `gamut-avif` wires these together for you end to end.
let img = AvifStillImage {
    width: 64,
    height: 64,
    bit_depth: 8,
    num_channels: 3,
    av1c: av1c_config,   // Av1cConfig: seq_profile, level, subsampling, ...
    nclx: nclx_colr,     // NclxColr: CICP code points + full_range
    item_data: &obus,    // AV1 temporal unit
};
let avif_bytes: Vec<u8> = write_avif_still(&img);
```

See [`gamut-avif`](../gamut-avif) for the full encode path that drives this crate.

## Status

M0 writes the minimal box set for a single lossless AVIF still image. The richer item set — alpha
(`auxl`), `grid`, transform properties, and sequence tracks (`moov`/`trak`), plus HEIC `hvc1`
support — is deferred per [`gamut-avif/STATUS.md`](../gamut-avif/STATUS.md).

## Roadmap

- Alpha auxiliary items, image transforms (`irot`/`imir`/`clap`), and grids.
- Sequence tracks for animated AVIF.
- HEIC/HEVC item support, shared with this crate.

## License

Licensed under either of MIT or Apache-2.0 at your option.
