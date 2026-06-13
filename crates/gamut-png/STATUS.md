# gamut-png — PNG encoder status

Tracking GitHub issue #24: a research-grade, space-efficient PNG **encoder**, on par with the best
PNG encoders. Delivered as small, individually green phases (each `just test`/`lint`/`format-check`/
`coverage` ≥80%).

**Keystone:** the signature → IHDR → IDAT → IEND pipeline with filter-None 8-bit RGB (P2) — once
libpng decodes that pixel-exact, each later phase swaps in another colour type, a filter, or a
space optimisation behind the same chunk spine and CRC.

**Oracle:** differential vs **libpng** (`tooling/libpng-oracle` + `third_party/libpng`, dev-only
FFI). gamut ships no PNG decoder, so the gate is: libpng decodes the encoder's output → pixel-exact
with the source; output size is benchmarked against libpng at maximum compression.

**Out of scope:** decoding (issue #24), Adam7 interlacing, animation/APNG (gamut is image-first).

## Phases

| Phase | Spec | Scope | Status |
| ----- | ---- | ----- | ------ |
| P1 | §5, §11.2.1 | Scaffold + workspace wiring + libpng-oracle/submodule; CRC-32; chunk writer + signature; `ColorType` + bit-depth matrix; IHDR | ✅ done |
| P2 | §6, §9, §11.2.4 | **Keystone:** `EncodeImage<Rgb8>`, filter None, DEFLATE → signature/IHDR/IDAT/IEND | ✅ done |
| P3 | §9 | All 5 scanline filters (None/Sub/Up/Average/Paeth) + `MinSumAbs` selection | ✅ done |
| P4 | §6.1 | Colour types: Gray8/Gray16/Rgb16/Rgba8/Rgba16/GrayAlpha8/16 (16-bit big-endian) | ✅ done |
| P5 | §11.2.2/§11.3.2 | Indexed (`encode_indexed8` + PLTE + tRNS), 8-bit | ✅ done |
| P6 | §7.2 | Sub-byte depths: 1-bit bilevel grey + auto-minimal-depth indexed (1/2/4) | ✅ done |
| P7 | §11.3 | Standard ancillary chunks: gAMA/cHRM/sRGB/sBIT/bKGD/pHYs/tIME/tEXt/zTXt/iTXt | ✅ done |
| P8 | §11.3 | Metadata: eXIf, iCCP (deflate-compressed), iTXt-XMP (raw-bytes setters) | ✅ done |
| P9 | §4.5 | **Space opt:** bit-depth/palette/gray/alpha/16→8 reduction; brute-force filter strategy; `Level::Best` | ⏳ todo |
| P10 | — | CLI `gamut convert → .png`; umbrella `png` feature; final API review | ⏳ todo |
