# gamut-metadata

`gamut-metadata` is the **unified image-metadata facade** for the gamut workspace. It brings the
per-format crates — [`gamut-exif`](../gamut-exif), [`gamut-xmp`](../gamut-xmp),
[`gamut-icc`](../gamut-icc), and [`gamut-iptc`](../gamut-iptc) — under one `Metadata` model and one
extract/embed surface.

## Design

- **One carrier, one field.** `Metadata` has exactly one field per genuinely distinct serialization
  a container holds: `exif`, `xmp`, `icc`. **IPTC has no field of its own** — IPTC Photo Metadata
  *is* XMP (properties in the `dc:`/`photoshop:`/`Iptc4xmp*` namespaces), so it lives inside `xmp`,
  read back through the `Metadata::iptc()` lens. Storing it separately would duplicate the same
  data. The one genuinely separate IPTC carrier, the legacy binary IIM block, is reconciled *into*
  the XMP graph on extract and projected back out only on request when embedding.
- **Container-agnostic.** It consumes already-located `MetadataBlock` byte payloads (from the WebP
  `EXIF`/`XMP `/`ICCP` chunks or the AVIF/HEIF `Exif`/`mime`/`colr` items) and produces owned
  `EncodedMetadata` blocks — it never parses boxes or chunks itself, keeping the
  `format → metadata` dependency thin.
- **Orchestration-only.** It holds the leaf crates' types by value and delegates all parsing and
  serialization to them; the leaf crates own correctness (and their own oracle test suites).
- **Cross-format reconciliation.** The two IPTC carriers — legacy IIM and IPTC-Core-in-XMP — are
  merged into the single XMP graph via `gamut-iptc`, resolving disagreements with a configurable
  `ConflictPolicy` (`conflicts()` reports them without resolving). Because each datum is stored once,
  the extract → embed → extract round-trip is a **true equality**.
- **Extensions are not a fourth carrier.** `Metadata::extensions` is a namespaced table for data no
  carrier can express, so a downstream typed model round-trips through `Metadata` without being
  narrowed to three fields. It is explicitly outside the carrier model: extraction never produces
  an extension and embedding never emits one.

## Extensions: data with no carrier

A downstream typed model is usually wider than what a still-image file can carry — sensor geometry,
container-level facts, structs it derives itself. `Metadata::extensions` holds that residue as
`MetadataExtension { namespace, key, value }`, where `namespace` is a reverse-DNS string or URI the
caller owns (the `gamut.` prefix is reserved) and `value` is the same TIFF/IFD `Value` model gamut's
metadata crates already use.

Two guarantees, deliberately distinct:

| | What survives | |
| --- | --- | --- |
| **Model round-trip** | carriers **and** extensions | `their model → Metadata → their model` |
| **Carrier round-trip** (keystone, unchanged) | `exif` / `xmp` / `icc` only | extract → embed → extract is still a true equality |

**Prefer a carrier whenever one exists** — only a carrier reaches the file. An unmodelled EXIF tag,
MakerNote included, already round-trips inside `exif` because `Exif` retains the raw `gamut_ifd::Ifd`;
any property round-trips inside `xmp` because the XMP graph is open; an unmodelled ICC element
round-trips inside `icc` as `TagData::Raw`. Reach for an extension only when no carrier can hold the
datum at all.

```rust
use gamut_metadata::{ExtensionPolicy, Metadata, MetadataEmbedder};
use gamut_metadata::exif::Value;

let mut meta = Metadata::default();
meta.set_extension("com.example.raw", "WhiteLevel", Value::Long(vec![16_383]));

// Embedding cannot carry it: dropped by default, or refused when losing it must be an error.
let blocks = meta.encode()?;                       // no block corresponds to the extension
let strict = MetadataEmbedder::new()
    .extension_policy(ExtensionPolicy::Reject)
    .embed(&meta);                                  // Err(MetadataError::UnembeddableExtension { .. })
```

Not yet supported, and deferred deliberately: a `MetadataBlock` variant for container-located blocks
the facade does not model, which would let *extraction* produce extensions. Today extensions are set
by the caller only.

## Usage

```rust
use gamut_metadata::{ConflictPolicy, Metadata, MetadataBlock, MetadataExtractor};

// A container crate has already located the metadata payloads in a file.
let meta = MetadataExtractor::new()
    .policy(ConflictPolicy::XmpWins)
    .extract(&[
        MetadataBlock::Exif(exif_payload),
        MetadataBlock::Xmp(xmp_payload),
        MetadataBlock::Icc(icc_payload),
        MetadataBlock::IptcIim(iptc_iim_payload), // legacy carrier, folded into `xmp`
    ])?;

// Typed IPTC access is a lens over `meta.xmp` — it stores nothing.
if let Some(iptc) = meta.iptc() {
    println!("city: {:?}", iptc.city());
}

// Serialize back to per-carrier blocks for a container to embed.
let blocks = meta.encode()?;
// blocks.exif / blocks.xmp / blocks.icc are Some(Vec<u8>) when their field was present.
```

The umbrella [`gamut`](../gamut) crate re-exports this crate as `gamut::metadata` behind its
`metadata` feature.

## Migrating from 1.x

`Metadata`, `MetadataBlock`, and `EncodedMetadata` are now `#[non_exhaustive]`, so a later carrier
is an additive change rather than a breaking one. Struct literals become a constructor call:

```rust
// 1.x
let meta = Metadata { exif, xmp, icc };
// 2.x
let meta = Metadata::from_carriers(exif, xmp, icc);
```

Use `Metadata::default()` plus field assignment when you also need `extensions`, and add a wildcard
arm to any exhaustive `match` on `MetadataBlock`. Nothing else changed: every existing method keeps
its signature and behaviour.

## Consumer integration

The format crates (`gamut-avif`/`gamut-webp`/`gamut-heic`/…) gaining a `gamut-metadata` dependency to
read, preserve, and embed metadata is tracked in their own milestones; the dependency direction
`format → gamut-metadata → per-format crates` is settled here.

## License

Licensed under either of MIT or Apache-2.0 at your option.
