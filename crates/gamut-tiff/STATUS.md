# gamut-tiff — TIFF 6.0 implementation status

Tracking GitHub issue #107: implement the **full TIFF 6.0 standard** (`references/tiff/tiff6.pdf`,
§1–23). Delivered as a stack of small, individually-reviewable PRs (P1–P19) onto the `feat/tiff`
integration branch; each PR is independently green (`just test`/`lint`/`format-check`/`coverage`
≥80%) and mergeable on its own.

**Keystone:** TIFF has no prediction/transform machinery — the hard part is the container
serialization spine (two-pass absolute-offset layout, ≤4-byte inline value packing, ascending-tag
sort, II/MM byte-order awareness). Once the uncompressed strip pipeline is **pixel-exact both
directions vs libtiff** (P3, gated by P4), each later phase just swaps a strip codec or photometric
mapping into the same spine.

**Oracle:** differential vs **libtiff** (dev-only FFI; `tooling/libtiff-oracle` +
`third_party/libtiff`). Lossless paths must agree **pixel-for-pixel** both directions (TIFF
permits many valid byte layouts, so the gate is pixel-exact, not byte-exact); JPEG-in-TIFF is
lossy → MAE/PSNR tolerance.

## Phases

| Phase | Spec § | Scope | Status |
| ----- | ------ | ----- | ------ |
| P1  | —       | Scaffold: crate, workspace wiring, docs, region-free skeleton | ✅ done |
| P2  | §2      | TIFF structure: header, IFD read/write, field types, value/offset packing | ✅ done |
| P3  | §3–4,6  | **Keystone** — uncompressed grayscale + RGB via strips; `Encoder`/`Decoder` | ✅ done |
| P4  | —       | libtiff oracle + pixel-exact both-direction differential gate | ☐ |
| P5  | §3,9    | Bilevel (1-bit) + 4-bit gray + PackBits | ☐ |
| P6  | §5      | Palette-color (ColorMap) | ☐ |
| P7  | §7–8    | Baseline field-reference hardening + CLI `convert → .tiff` | ☐ |
| P8  | §10     | Modified Huffman (Compression=2) | ☐ |
| P9  | §13     | LZW (Compression=5) | ☐ |
| P10 | §14     | Differencing predictor (Predictor=2) | ☐ |
| P11 | §11     | CCITT T.4 / T.6 fax (Compression=3/4) | ☐ |
| P12 | §15     | Tiled images | ☐ |
| P13 | §18–19  | Planar config + associated alpha + sample format (16-bit/float) | ☐ |
| P14 | §16     | CMYK | ☐ |
| P15 | §21     | YCbCr | ☐ |
| P16 | §20,23  | RGB colorimetry + CIE L\*a\*b\* | ☐ |
| P17 | §12,17  | Multi-page documents + halftone hints | ☐ |
| P18 | §22     | JPEG-in-TIFF (Compression=7) — deferrable tail | ☐ |
| P19 | —       | Finalization: robustness corpus, interop sweep, docs | ☐ |
