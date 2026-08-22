JPEG XL standard is formalized by ISO/IEC 18181. The 4-part standard (Core coding system, File Format, Conformance Testing, Reference Software) would be the definitive ground truth for all implementations of JXL but as it is paywalled, we will use the reference implementation [libjxl](https://github.com/libjxl/libjxl) the oracle.

Some free references have been vendored in this directory and should be mostly sufficient along side the reference implementation:

- `libjxl` format_overview.md: <https://raw.githubusercontent.com/libjxl/libjxl/refs/heads/main/doc/format_overview.md>
- The Committee Draft of Part 1 (core spec before ISO finalization): <https://arxiv.org/pdf/1908.03565>

## Version pins (gamut-jxl)

The [`gamut-jxl`](../../crates/gamut-jxl) crate pins the reference implementations exactly, and these
versions are the ground truth its tests are gated against:

- **libjxl v0.12.0** — both the conformance **oracle** and the vendored **encoder core**. It is
  statically built via the BSD-3-Clause [`jpegxl-src`](https://crates.io/crates/jpegxl-src) crate,
  pinned `= 0.12.0`, and linked through [`gamut-jxl-sys`](../../crates/gamut-jxl-sys) (which also
  exposes libjxl's decoder as the differential-test oracle).
- **jxl-rs (`jxl` crate) v0.4.3** — the pure-Rust **decode** implementation.

Bumping either is a **deliberate** change, not a routine dependency update: it must re-run the
`gamut-jxl` differential suite and the `gamut-jxl-sys` version-pin / symbol-drift tests
(`tests/version.rs`, which assert the linked libjxl reports version `12000`), because the
hand-written FFI declarations are transcribed against the pinned libjxl headers and drift silently
otherwise.

## Vendored opsin constants

`opsin_params.h` is libjxl 0.12.0's `lib/jxl/cms/opsin_params.h` verbatim (BSD-3-Clause, header
retained): the frozen XYB opsin absorbance matrix, bias, inverse matrix, and scaled-XYB byte
encoding. Transcribed (with the derivation notes) into `references/color/README.md` for
`gamut-color`'s `xyb` module and `gamut-jpeg`'s XYB colour mode.
