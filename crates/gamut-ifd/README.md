# gamut-ifd

`gamut-ifd` is a pure-Rust implementation of the **TIFF Image File Directory (IFD) container core**:
the byte-order header, the field-type / value model, the IFD chain, and the offset-driven read/write
spine. It models *structure only* — no pixels, compression, or photometry.

## Goals

Part of the [gamut](../../README.md) workspace, this crate exists because the TIFF/IFD structure is
shared by two otherwise-separate efforts:

- **EXIF** ([`gamut-exif`](../gamut-exif), issue #34) — an EXIF blob is an `Exif\0\0` marker followed
  by a TIFF stream; its 0th/1st IFDs and Exif/GPS/Interop sub-IFDs are exactly IFD chains.
- **TIFF** ([`gamut-tiff`](../gamut-tiff), issue #107) — the TIFF image codec, whose container *is*
  its IFD structure.

Factoring the IFD core out keeps the two from duplicating the fiddly, security-sensitive offset
machinery. It is:

- **Memory-safe on hostile input.** `#![forbid(unsafe_code)]` — IFDs are offset-driven, a classic
  parser-exploit surface (offset loops, truncation, overlapping extents).
- **Endianness-honest.** TIFF carries its own byte order (II/MM); the [`ByteOrder`] is threaded
  through every access rather than fixed at compile time.
- **Dependency-light.** Builds only on [`gamut-core`](../gamut-core).

The public types deliberately mirror `gamut-tiff`'s structural types: the codec was migrated onto
this crate as a near-zero-diff refactor (issue #107), and now consumes it instead of an inlined copy.

## Usage

`read` / `read_header` parse a stream into a [`TiffFile`] (`ByteOrder` + `Variant` + a `Vec<Ifd>`);
`write` serialises one back, handling the two-pass offset layout. Each `Ifd` is a tag-sorted set of
`Field`s, each holding a typed [`Value`]; `FieldType` carries the on-disk type codes.

```rust
use gamut_ifd::{ByteOrder, Ifd, TiffFile, Value, Variant, read, write};

let mut ifd = Ifd::new();
ifd.set(256, Value::Short(vec![640])); // ImageWidth
ifd.set(257, Value::Short(vec![480])); // ImageLength
let file = TiffFile { order: ByteOrder::LittleEndian, variant: Variant::Classic, ifds: vec![ifd] };
let bytes = write(&file);
assert_eq!(read(&bytes).unwrap(), file);
```

Tag *numbers* are passed literally — tag *semantics* live in the consuming codec (e.g. `gamut-tiff`'s
`tags` module), not in this structural core.

### BigTIFF

The `bigtiff` cargo feature adds BigTIFF (`references/tiff/bigtiff.html`): the `Variant::Big`
container with 64-bit offsets/counts and the `Long8` / `SLong8` / `Ifd8` field types. It is additive
and off by default — classic-only consumers (EXIF) stay lean; `gamut-tiff` enables it.

## Status

Structural core implemented (issue #107). The EXIF-specific layers (sub-IFD traversal, fuzz corpus)
remain under issue #34. See [STATUS.md](STATUS.md).

## License

Licensed under either of MIT or Apache-2.0 at your option.
