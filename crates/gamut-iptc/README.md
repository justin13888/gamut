# gamut-iptc

`gamut-iptc` is a pure-Rust **IPTC photo metadata** parser and serializer, covering both the legacy
IIM form and the modern IPTC Photo Metadata (Core + Extension) carried over XMP.

## Goals

Part of the [gamut](../../README.md) workspace, this crate reads, preserves, and embeds IPTC photo
metadata. It is:

- **Memory-safe on hostile input.** `#![forbid(unsafe_code)]`.
- **Clean-slate from the spec.** Implemented from **IPTC-IIM 4.2** and the **IPTC Photo Metadata
  Standard** ([`../../references/iptc`](../../references/iptc)).
- **Two carriers, one model.** Legacy IIM is a binary dataset stream inside a Photoshop Image
  Resource Block (`8BIM`, resource `0x0404`); the modern Core/Extension fields *are* XMP, so that
  path builds on [`gamut-xmp`](../gamut-xmp). The hard part is reconciling the two when both are
  present.

## Usage

The legacy IIM carrier is implemented end to end — parse and serialize the Photoshop `8BIM`
resource stream (`PhotoshopIrb`), the IIM dataset stream (`IimBlock` / `IimDataSet`), and decode
text with the coded character set (`IimCharset`):

```rust
use gamut_iptc::{IimBlock, IimCharset, IimDataSet, IptcReader, PhotoshopIrb};

let block = IimBlock {
    datasets: vec![
        IimDataSet { record: 2, dataset: 0, data: vec![0, 4] },        // Record Version = 4
        IimDataSet { record: 2, dataset: 25, data: b"sky".to_vec() },  // Keywords
    ],
};
let irb = PhotoshopIrb::with_iptc(block.encode()?).encode()?;          // 8BIM 0x0404 resource
let read = IptcReader::new().read_irb(&irb)?.expect("0x0404 present");
let charset = IimCharset::detect(&read)?;                              // 1:90, default Latin-1
assert_eq!(charset.decode(&read.datasets[1].data)?, "sky");
# Ok::<(), gamut_core::Error>(())
```

`IimTagInfo::lookup(record, dataset)` resolves a dataset's name and wire constraints.

The modern IPTC Photo Metadata path models Core fields as XMP properties (`PhotoMetadata`, with typed
accessors for creator, location, rights, keywords, …). `IptcReader::read` merges the two carriers
into one view, and `IptcWriter` projects a view back to the legacy carrier:

```rust
use gamut_iptc::{IimBlock, IimDataSet, IptcReader, IptcWriter, ConflictPolicy};

let iim = IimBlock { datasets: vec![
    IimDataSet { record: 2, dataset: 90, data: b"Lyon".to_vec() }, // City
] };
// When IIM and XMP disagree, the policy decides; XMP wins by default.
let merged = IptcReader::new().policy(ConflictPolicy::IimWins).read(Some(&iim), None);
assert_eq!(merged.city(), Some("Lyon"));

// ...and back: PhotoMetadata -> 8BIM resource stream (None if there is nothing to embed).
let irb = IptcWriter::new().write_irb(&merged)?.expect("City is present");
# Ok::<(), gamut_core::Error>(())
```

`gamut-iptc` operates on the in-memory XMP property graph; parsing/serializing the XMP *packet bytes*
is [`gamut-xmp`](../gamut-xmp)'s responsibility (issue #34). IPTC **Extension** structures are out of
scope; the typed accessors cover IPTC **Core**.

## Status

Legacy IIM + Photoshop IRB, IPTC Core over XMP, and IIM↔XMP reconciliation: **implemented**. The
differential exiv2 oracle and gamut-metadata facade wiring follow. See [STATUS.md](STATUS.md).

## License

Licensed under either of MIT or Apache-2.0 at your option.
