# gamut-isobmff

`gamut-isobmff` is a pure-Rust implementation of the **ISO Base Media File Format (ISOBMFF)
still-image container core**: the `ftyp` brands, the `meta` box of image items with their properties
and payloads, and the offset-driven read/write spine. It models *structure only* — the coded
bitstream (the `av1C`/`hvcC` record and the sample data) stays opaque.

## Goals

Part of the [gamut](../../README.md) workspace, this crate exists because the ISOBMFF/HEIF container
is shared by two otherwise-separate codecs:

- **AVIF** ([`gamut-avif`](../gamut-avif)) — AV1 still images: item type `av01`, codec config `av1C`.
- **HEIC** ([`gamut-heic`](../gamut-heic)) — HEVC still images: item type `hvc1`, codec config `hvcC`.

Factoring the container out keeps the two from duplicating the box tree and the fiddly,
security-sensitive `iloc` offset machinery. It is:

- **Codec-agnostic.** The codec configuration is carried as opaque bytes
  ([`PropertyKind::CodecConfiguration`]), so the same `write`/`read` serve `av01`/`av1C` and
  `hvc1`/`hvcC` with no container changes.
- **Memory-safe on hostile input.** `#![forbid(unsafe_code)]` — ISOBMFF is offset-driven, a classic
  parser-exploit surface (truncation, overruns, out-of-range indices), so every read is
  bounds-checked.
- **Dependency-light.** Builds only on [`gamut-core`](../gamut-core).

## Usage

`write` serialises an [`IsoBmffImage`] (its `ftyp` brands, the `pitm` primary item, and the image
items) into a complete `ftyp` + `meta` + `mdat` file; `read` parses one back. Each [`Item`] carries
its type, name, properties, and payload; the writer derives the `iloc` offsets and the shared
`ipco`/`ipma` so the two are inverse for any file this crate writes.

```rust
use gamut_isobmff::{IsoBmffImage, Item, Property, PropertyKind, read, write};

let img = IsoBmffImage {
    major_brand: *b"avif",
    minor_version: 0,
    compatible_brands: vec![*b"avif", *b"mif1", *b"miaf"],
    primary_item_id: 1,
    items: vec![Item {
        id: 1,
        item_type: *b"av01",
        name: String::new(),
        properties: vec![Property {
            essential: false,
            kind: PropertyKind::ImageSpatialExtents { width: 64, height: 64 },
        }],
        payload: vec![/* the coded bitstream, opaque to this crate */],
    }],
};
let bytes = write(&img);
assert_eq!(read(&bytes).unwrap(), img);
```

See [`gamut-avif`](../gamut-avif) for the full encode path that drives this crate (it builds the
`av1C` record and the AVIF brand set, then calls `write`).

## Status

Models the HEIF still-image box set: `ftyp`, `meta` (`hdlr`/`pitm`/`iloc` v0/`iinf`+`infe` v2/`iprp`),
the `ispe`/`pixi`/`colr`/`irot`/`imir` properties, opaque codec configuration, and `mdat`.
Unrecognised property boxes round-trip verbatim. Image sequences/tracks, `iloc` v1/v2, multi-extent
items, `idat`/`grid`/alpha, and ICC `colr` are out of scope — see [STATUS.md](STATUS.md).

Box byte layouts follow ISO/IEC 14496-12 (ISOBMFF) and ISO/IEC 23008-12 (HEIF) — paywalled, so
cross-checked against the public AVIF box table and a vendored libavif/dav1d differential oracle
(via [`gamut-avif`](../gamut-avif)). See [`references/isobmff`](../../references/isobmff).

## License

Licensed under either of MIT or Apache-2.0 at your option.
