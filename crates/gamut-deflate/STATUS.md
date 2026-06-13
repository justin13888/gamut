# gamut-deflate — DEFLATE / zlib encoder status

Tracking GitHub issue #24 (PNG): a pure-Rust, space-efficient DEFLATE (RFC 1951) + zlib (RFC 1950)
**encoder**, the compression primitive `gamut-png` builds on. Delivered as small, individually green
phases (each `just test`/`lint`/`format-check`/`coverage` ≥80%).

**Keystone:** the bit-exact stored/fixed-Huffman + zlib-framing path (P-D2) — once that round-trips
through the reference inflater, every later phase swaps in a smarter block coder or parse behind the
same framing and Adler-32/FCHECK spine, with the oracle proving each step still decodes.

**Oracle:** differential vs the canonical C **zlib** (`tooling/zlib-oracle` + `third_party/zlib`,
dev-only FFI). gamut ships no inflater, so the gate is: inflate the encoder's output → byte-exact
with the input; size is benchmarked against zlib's own `compress2` at the matching level.

**Decoding is out of scope** (issue #24): inflating DEFLATE is solved; this crate only encodes.

## Phases

| Phase | Spec | Scope | Status |
| ----- | ---- | ----- | ------ |
| P-D1 | RFC 1950/1951 | Scaffold + workspace wiring + zlib-oracle/submodule; LSB `BitWriter`; Adler-32; zlib header (CMF/FLG/FCHECK); **stored blocks** (the always-correct floor) | ✅ done |
| P-D2 | §3.2.6 | Fixed-Huffman blocks (literals); per-input choice of stored vs fixed | ✅ done |
| P-D3 | §3.2.5, §4 | Length/distance symbol tables (exhaustive inversion tests); greedy hash-chain LZ77 matcher under fixed Huffman | ✅ done |
| P-D4 | §3.2.2, §3.2.7 | **Dynamic Huffman**: length-limited code build + canonical codes + code-length RLE (16/17/18); per-block min-cost selection; lazy matching → `Level::Default` | ⏳ todo |
| P-D5 | §4 | Entropy-cost block splitting | ⏳ todo |
| P-D6 | — | **Optimal parse** (zopfli-style DP + iterative entropy model) + package-merge 15-bit length limiting → `Level::Best` | ⏳ todo |

## Notes / deferred

- **LSB `BitWriter`** is currently vendored here (ported from `gamut-webp`'s VP8L writer). If a third
  consumer appears, graduating a shared writer into `gamut-bitstream` is a clean follow-up.
- The public API surface (`DeflateEncoder`, `Level`, `adler32`) is stable from P-D1; later phases
  only improve the ratio, never change behaviour observable through the inflater.
- Until the later phases land, every `Level` falls back to stored blocks — correct, but not yet
  compressed. The oracle round-trip tests already cover all levels.
