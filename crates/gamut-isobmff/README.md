# gamut-isobmff

`gamut-isobmff` is a pure-Rust implementation of the **ISO Base Media File Format (ISOBMFF)
still-image container core**: the `ftyp` brands, the `meta` box of image items with their
properties, references, and payloads, and the offset-driven read/write spine. It models *structure
only* — the coded bitstream (the `av1C`/`hvcC` record and the sample data) stays opaque.

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
  parser-exploit surface (truncation, overruns, out-of-range indices, amplification), so every read
  is bounds-checked and the total resolved payload is capped at the input size.
- **Validating on output.** `write` returns a typed error for a model that cannot round-trip or
  does not fit the still-image box versions, instead of silently truncating fields.
- **Dependency-light.** Builds only on [`gamut-core`](../gamut-core).

## Usage

`write` serialises an [`IsoBmffImage`] (its `ftyp` brands, the `pitm` primary item, the image
items, and any entity groups) into a complete `ftyp` + `meta` + `mdat` file; `read` parses one
back. Each [`Item`] carries its type, name, optional MIME info, hidden flag, typed `iref`
references (`auxl`/`cdsc`/`dimg`/`thmb`/`prem`, …), properties, and payload; the writer derives the
`iloc` offsets and the shared `ipco`/`ipma` so the two are inverse for any file this crate writes.

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
        content_type: None,
        content_encoding: None,
        hidden: false,
        references: vec![],
        properties: vec![Property {
            essential: false,
            kind: PropertyKind::ImageSpatialExtents { width: 64, height: 64 },
        }],
        payload: vec![/* the coded bitstream, opaque to this crate */],
    }],
    groups: vec![],
};
let bytes = write(&img)?;
assert_eq!(read(&bytes)?, img);
```

See [`gamut-avif`](../gamut-avif) for the full encode path that drives this crate (it builds the
`av1C` record and the AVIF brand set, then calls `write`).

## Inspect and re-mux from the CLI

The `gamut` CLI (with the `isobmff` feature, on in `all`) exercises this crate's whole read/write
surface on real files — no working codec required, since the coded bitstream is opaque:

```console
# parse a still-image .avif/.heic and print its box structure (brands, items, properties,
# references, grid geometry, entity groups)
$ gamut isobmff inspect image.avif

# re-serialise a container (normalised box versions, single-extent mdat); the coded payload is
# preserved verbatim, so a real decoder re-decodes the result to identical pixels
$ gamut isobmff remux image.avif out.avif

# build a synthetic container exercising every modelled box, property, reference and group
$ gamut isobmff build demo.avif
```

`inspect`/`remux` accept the foreign-encoder repertoire below; out-of-scope structures — image
sequences (`moov`/`trak`), `largesize`/size-0 boxes — and malformed input are rejected with a typed
error rather than mis-parsed (this crate is `#![forbid(unsafe_code)]` and bounds-checks every read).

## Status

Models the HEIF still-image box set: `ftyp`, `meta` (`hdlr`/`pitm`/`iloc`/`iinf`/`iref`/`iprp`/
`idat`/`grpl`), the `ispe`/`pixi`/`colr` (`nclx` + ICC)/`irot`/`imir`/`clap`/`pasp`/`auxC`/`clli`
properties, opaque codec configuration, and `mdat`. Unrecognised property boxes round-trip
verbatim. The writer normalises to the smallest box versions; the reader additionally accepts the
foreign-encoder repertoire (`iloc` v1/v2, `idat` placement, multi-extent payloads, 32-bit item
ids, 16-bit `ipma` indices). Image sequences/tracks, item protection, and external data references
are out of scope — see [STATUS.md](STATUS.md) for the full deferred/out-of-scope ledger.

Box byte layouts follow ISO/IEC 14496-12 (ISOBMFF) and ISO/IEC 23008-12 (HEIF) — paywalled, so
cross-checked against the public AVIF box table, hand-authored spec fixtures, and a vendored
libavif/dav1d differential oracle (via [`gamut-avif`](../gamut-avif)). See
[`references/isobmff`](../../references/isobmff).

## License

Licensed under either of MIT or Apache-2.0 at your option.
