# gamut-wasm

`gamut-wasm` provides WebAssembly bindings for the gamut image codecs, exposing the encoders to
JavaScript and TypeScript so they can run directly in the browser.

## Goals

Part of the [gamut](../../README.md) workspace, this crate exists to:

- **Make WASM a first-class target.** A native Rust → wasm build is smaller and faster to
  instantiate than the C codecs run through Emscripten, and it talks to the JS/TS ecosystem directly
  — making serverless/edge image optimization (Workers, Lambda, and friends) practical.
- **Expose the encoders, not re-implement them.** It is a thin `wasm-bindgen` wrapper over the
  umbrella [`gamut`](../gamut) crate; all encoding stays in the gamut codecs.
- **Stay memory-safe.** `#![forbid(unsafe_code)]` — the `wasm-bindgen` glue needs no `unsafe`.

This crate builds as a `cdylib`.

## Usage

No public API yet — implementation pending. Once it lands, build it with `wasm-pack` (or
`wasm-bindgen`) and call the generated bindings from JS/TS — roughly:

```js
import init, { encodeAvif } from "gamut-wasm";

await init();
const avifBytes = encodeAvif(rgba, width, height);
```

## Status

Placeholder — implementation pending.

## Roadmap

- `wasm-bindgen` entry points for the implemented encoders (AVIF first), with a `wasm-pack` build.
- Typed JS/TS API and a published npm package.

## License

Licensed under either of MIT or Apache-2.0 at your option.
