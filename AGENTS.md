# gamut

A collection of space-efficient image encoding libraries.

## Packages

- **tracing** -- structured diagnostic logging emitted by the encoders; no subscriber is
  configured here (this is a library — the consuming application installs one).
- **thiserror** -- derives `std::error::Error` for the crate's public error enums so callers
  get ergonomic, typed encoding/decoding failures.

## Quality

Validate changes:

```bash
just test            # correctness
just format-check    # formatting
just lint            # lint (Clippy, warnings as errors)
just coverage        # coverage (minimum 80%)
```

## Conventions

- All `pub` items need doc comments. Mark fallible/owning return types with `#[must_use]`
  where dropping the value is likely a bug.
- No `unwrap()`/`expect()` in library code paths — return typed errors via `thiserror`.
- Keep encoders allocation-conscious: prefer slices and `&[u8]` over owned buffers in hot
  paths, and document the space/time tradeoff of each format.
