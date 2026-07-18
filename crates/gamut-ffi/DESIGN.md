# gamut-ffi C API design contract

The target specification for the C API of the gamut codecs (issue #242). The conversion itself —
largely automated from the Rust API, per-crate — has not started; this document fixes the
conventions that conversion tooling generates against and that the Rust API is kept compatible
with (see the "C portability" convention in the repository `AGENTS.md`).

Why a C API rather than C++: Rust's C ABI compatibility is first-class, and C++ applications can
consume a well-designed C API equally idiomatically; maintaining good C++ API practice would mean
downgrading idiomatic Rust patterns anyway. Distribution is a ZIP of compiled C libraries built
from this crate (see [Packaging](#packaging)).

## Naming

- Everything is prefixed `gamut_` (types/constants `Gamut`/`GAMUT_`).
- Functions: `gamut_<crate>_<noun>_<verb>` — e.g. `gamut_png_encoder_new`,
  `gamut_png_encoder_free`, `gamut_jpeg_decoder_set_max_dimensions`.
- Opaque types: Rust `PngEncoder` → `GamutPngEncoder`, exposed in the header as
  `typedef struct GamutPngEncoder gamut_png_encoder_t;`.
- Enum constants: `GAMUT_<TYPE>_<VARIANT>` — e.g. `GAMUT_PIXEL_FORMAT_RGB8`.
- Headers are always **generated from the Rust source of truth**, never handwritten. Whether
  generation is cbindgen or emitted by the conversion tooling itself is an open item, decided at
  conversion kickoff.

## Opaque handles

Stateful or Rust-rich types — encoders, decoders, rich decode results (`DecodedPng`-class) —
cross the boundary as opaque pointers with `_new`/`_free` lifecycles. Rust `with_*` builder
methods map to `gamut_*_set_*` functions returning a status code.

Plain-data types cross **by value**: `gamut_core::Dimensions` (`#[repr(C)]`, width then height),
`gamut_core::PixelFormat` and `ColorModel` (`#[repr(u32)]`, explicit permanent discriminants),
and any format-crate enum once it is repr-pinned during its conversion pass.

## Buffers

All byte buffers are pointer + length pairs (`const uint8_t *data, size_t len`). An image
crosses as `(samples_ptr, samples_len, gamut_dimensions_t, gamut_pixel_format_t)` and is
validated at the boundary with exactly the semantics of `ImageRef::new` (length =
`width * height * channels`, non-empty dimensions).

## Errors

Status codes are `int32_t` (`gamut_status_t`): `GAMUT_OK = 0`; nonzero codes map
`gamut_core::Error` variants (`GAMUT_STATUS_INVALID_INPUT`, `GAMUT_STATUS_UNSUPPORTED`,
`GAMUT_STATUS_IO`) plus boundary-only codes (`GAMUT_STATUS_NULL_ARGUMENT`,
`GAMUT_STATUS_PANIC`, `GAMUT_STATUS_BUFFER_TOO_SMALL`). `Error` is `#[non_exhaustive]`; unknown
future variants map to a catch-all code.

Error messages: `Error::InvalidInput`/`Error::Unsupported` carry `&'static str`, so their
messages are returned as **borrowed `const char*` valid for the program lifetime** — no
allocation, no free protocol. Preserving the `&'static str` payload property in `gamut_core` is
therefore a convention, not an accident. Dynamic messages (`Error::Io`) go through a
thread-local last-error string.

## Ownership

- Allocator symmetry: memory allocated by Rust is freed only by the matching `gamut_*_free`;
  C never `free()`s Rust memory and Rust never frees C memory.
- Every `_free` is null-tolerant (a no-op on `NULL`).
- Encode output is a Rust-owned byte buffer returned via out-params
  (`uint8_t **out, size_t *out_len`) and released with `gamut_bytes_free` — this matches the
  Rust contract of appending to a caller `Vec<u8>`.
- The two-call size/fill pattern (query size with `NULL`, then fill caller memory) is reserved
  for fixed-size or queryable payloads (decoded-image probes, metadata strings), not encode
  output.

## Panic policy

Every `extern "C"` entry point wraps its body in `catch_unwind`; a panic becomes
`GAMUT_STATUS_PANIC` and never unwinds across the boundary.

## Trait mapping

- **Consumer boundary (C calls Rust)** — the part tooling generates: each format crate's
  object-safe `EncodeImage<P>` / `DecodeImage<P>` impls are exported per
  (codec, `PixelFormat`) pair, enumerated mechanically from `gamut_core::PixelFormat::ALL` ×
  the crate's impl set (e.g. `gamut_png_encoder_encode_rgb8`, or a single entry point
  dispatching on a `gamut_pixel_format_t` argument — chosen at conversion kickoff, uniformly).
- **Provider boundary (C implements Rust traits)** — deferred to **`gamut-codec-abi`**
  (issue #241, sub-issue #272): its `#[repr(C)]` descriptors/vtables and `Foreign*` bridges are
  the sanctioned mechanism for foreign implementations plugging into gamut, including
  C-implemented hooks such as `gamut_heic::HevcDecoder` and future codestream backends. This
  crate must not grow a parallel vtable convention. `HevcDecoder` remains the *shape* template
  for new Rust hooks: a single object-safe method, borrowed bytes in, owned plain data out.

## Packaging

The release ZIP artifacts are builds of this one crate at chosen feature sets — the feature
table (strictly synced to `gamut`'s, enforced by `mise run check-ffi-features`) is the packaging
matrix:

- The fat artifact is a `--features all` build: `cdylib` + `staticlib` + generated header.
- Slim per-format or per-group variants are builds of the same crate with a feature subset —
  no per-format crates.
- Static consumers of the fat `staticlib` get dead-code elimination at their final link; the
  `cdylib` trades disk size for a single shared, demand-paged library.

## First-pass scope

In: the core `EncodeImage`/`DecodeImage` trait pair and the builder configs of each format
crate. Excluded initially (revisited per crate as conversion proceeds): rich ad-hoc surfaces
(`DecodedPng`/`PngImage`, jpeg `info()`/`metadata()`, HEIC item/metadata detail), the tiff/ifd
low-level writer functions, and the `primitives` re-exports. The C surface is feature-gated
identically to `gamut`.
