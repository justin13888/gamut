# gamut-dsp

`gamut-dsp` holds the shared digital signal processing routines for the gamut codecs — the
transforms, quantization, and filtering the format crates call into.

## Goals

Part of the [gamut](../../README.md) workspace, this crate exists to:

- **Keep the math in one place.** Transforms and related DSP are implemented and tested once here,
  rather than re-derived inside each codec, so a numerical bug is fixed for every format at once.
- **Track the spec exactly.** Routines are implemented clean-slate from the official specs (see
  [`../../references/`](../../references)) — the M0 pair is the AV1 lossless 4×4 Walsh–Hadamard
  transform ([`fwht4x4`] / [`iwht4x4`]), the exact inverse the AV1 decoder expects.
- **Stay allocation-conscious.** The transforms operate on fixed-size `[i32; 16]` arrays with no
  heap allocation in the hot path.
- **Stay memory-safe.** `#![forbid(unsafe_code)]`.

## Usage

```rust
use gamut_dsp::{fwht4x4, iwht4x4};

// The forward/inverse 4x4 Walsh-Hadamard transform round-trips exactly (lossless).
let residual = [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
let coeffs = fwht4x4(&residual);
let restored = iwht4x4(&coeffs);
assert_eq!(residual, restored);
```

## Status

M0 provides the lossless 4×4 Walsh–Hadamard transform pair used by AV1 lossless intra coding. The
discrete cosine / asymmetric discrete sine transforms used by *lossy* AV1 coding (AV1
§7.13.2.2–.9), plus quantization and in-loop filtering, are deferred to milestone M1 (see
[`gamut-avif/STATUS.md`](../gamut-avif/STATUS.md)).

## Roadmap

- M1: lossy DCT/ADST transform family and quantization.
- Later: filtering primitives and (where it pays off) SIMD-optimized variants.

## License

Licensed under either of MIT or Apache-2.0 at your option.
