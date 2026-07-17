# AV1 (AOMedia Video 1 — still-image intra subset)

Reference for **`gamut-av1`** — the pure-Rust AV1 still-image encoder, and the codec layer beneath
[`gamut-avif`](../avif). gamut implements AV1 **clean-slate from the specification** rather than
wrapping libaom, so the coded bitstream is validated *against* the reference codec, not produced by
it.

## Vendored

- **AV1 Bitstream & Decoding Process Specification** — [`av1-spec.pdf`](./av1-spec.pdf). The
  normative source for OBU framing, sequence/frame headers, tiling/partition/prediction, the
  transforms, quantization, entropy coding and the in-loop filters. Module doc comments in
  `gamut-av1` cite its section numbers (§5.x/§7.x/§8.x/§9.x).
- **AV1 Codec ISO Media File Format Binding v1.3.0** — [`av1-isobmff/`](./av1-isobmff). The `av01`
  item type and the `AV1CodecConfigurationRecord` (§2.3) that `gamut-avif` stamps into the `av1C`
  property; the temporal-unit payload rules (§2.4) for the `mdat`. Since issue #250 the record is
  also **read** (`gamut_avif::Av1Config`, §2.3.3 field-for-field with reserved bits ignored and
  the §2.3.4 `configOBUs` size-field SHALL enforced), and item payloads are enumerated
  container-side as OBUs (`gamut_avif::iter_obus`: the low-overhead syntax of AV1 §5.3 with the
  §4.10.5 `leb128()` bounds — padded encodings accepted — and the §2.4 rule that only the final
  OBU may omit its size field).

## Oracle — libaom (the AV1 reference codec)

The single golden oracle is **libaom**, AOMedia's **reference implementation** of AV1 — the most
authoritative encoder *and* decoder. It is vendored as the `third_party/aom` submodule, pinned to a
stable tag (**v3.14.1**), and built into a static library by the dev-only `tooling/aom-oracle`
crate (cmake + ninja; nasm for the x86 SIMD is auto-vendored). It exposes both directions, which
together validate gamut end-to-end:

- **Decoder → validates gamut's encoder (today).** `gamut-av1/tests/recon.rs` encodes a still with
  gamut, then decodes the raw AV1 temporal unit with libaom's reference decoder and asserts the
  result equals the encoder's own reconstruction buffer **byte-for-byte**. Because a conformant
  decoder reproduces exactly what a spec-legal bitstream prescribes, this is the definitive
  correctness gate for the encoder.
- **Encoder → validates the future gamut decoder.** gamut is encoder-first and has no AV1 decoder
  yet ([`../../crates/gamut-avif/STATUS.md`](../../crates/gamut-avif/STATUS.md)); libaom's reference
  **encoder** (`aom-oracle::encode_still_intra`) is the reference bitstream source that a future
  gamut decoder will be checked against. It is exercised today by `aom-oracle`'s own lossless
  encode→decode round-trip self-test.

### dav1d — corroborating secondary decoder

**dav1d** (VideoLAN's fast, independent AV1 decoder; `third_party/dav1d`, pinned to **1.5.3**) runs
alongside libaom in `recon.rs`: every case must decode identically under *both*. Two independent
decoders agreeing byte-for-byte is a strictly stronger conformance signal than either alone. dav1d
is also libavif's decode backend in the AVIF container cross-check
([`../avif`](../avif/README.md)), so it stays regardless.

### Not vendored

**SVT-AV1** and **rav1e** are alternative production/Rust AV1 encoders. They are relevant only to
optional *quality/performance* benchmarking of gamut's encoder (rate–distortion competitiveness),
not to conformance, and rate-control tuning is future work — so they are deliberately **not**
vendored here. No system-installed codec binaries are used anywhere.
