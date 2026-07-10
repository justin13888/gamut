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

## Consumer integration

The format crates (`gamut-avif`/`gamut-webp`/`gamut-heic`/…) gaining a `gamut-metadata` dependency to
read, preserve, and embed metadata is tracked in their own milestones; the dependency direction
`format → gamut-metadata → per-format crates` is settled here.

## License

Licensed under either of MIT or Apache-2.0 at your option.
