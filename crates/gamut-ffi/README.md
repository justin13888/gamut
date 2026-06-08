# gamut-ffi

`gamut-ffi` provides C-compatible FFI bindings for the gamut image codecs, so `gamut` can be used as
a drop-in replacement for traditional C image libraries from C, C++, Python, Go, and more.

## Goals

Part of the [gamut](../../README.md) workspace, this crate exists to:

- **Expose a stable C ABI.** A `extern "C"` surface over the umbrella [`gamut`](../gamut) crate, so
  the memory-safe Rust encoders are callable from any language with a C FFI.
- **Wrap, not re-implement.** All encoding stays in the gamut codecs; this crate only marshals across
  the boundary.
- **Contain `unsafe` to the boundary.** Unlike the rest of the workspace, `unsafe` is *permitted*
  here — but only for the `extern "C"` layer (raw pointers, lengths); the safe Rust core underneath
  keeps its `#![forbid(unsafe_code)]` guarantees.

This crate builds as both a `cdylib` and a `staticlib`, and will ship a generated C header.

## Usage

No public API yet — implementation pending. Once it lands, link against the `cdylib`/`staticlib` and
include the generated header — roughly:

```c
#include "gamut.h"

uint8_t *out = NULL;
size_t out_len = 0;
int rc = gamut_encode_avif(rgb, width, height, &out, &out_len);
/* ... use out[0..out_len] ... */
gamut_free(out, out_len);
```

## Status

Placeholder — implementation pending.

## Roadmap

- `extern "C"` entry points for the implemented encoders (AVIF first) with explicit allocation/free.
- A generated C header (e.g. via `cbindgen`) and packaging as `cdylib`/`staticlib`.

## License

Licensed under either of MIT or Apache-2.0 at your option.
