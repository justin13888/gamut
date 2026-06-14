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
use gamut_iptc::{IimBlock, IimCharset, IimDataSet, IptcReader, IptcWriter};

let block = IimBlock {
    datasets: vec![
        IimDataSet { record: 2, dataset: 0, data: vec![0, 4] },        // Record Version = 4
        IimDataSet { record: 2, dataset: 25, data: b"sky".to_vec() },  // Keywords
    ],
};
let irb = IptcWriter::new().write_irb(&block)?;                        // 8BIM 0x0404 resource
let read = IptcReader::new().read_irb(&irb)?.expect("0x0404 present");
let charset = IimCharset::detect(&read)?;                              // 1:90, default Latin-1
assert_eq!(charset.decode(&read.datasets[1].data)?, "sky");
# Ok::<(), gamut_core::Error>(())
```

`tag_info(record, dataset)` resolves a dataset's name and wire constraints. The modern IPTC Photo
Metadata (Core/Extension) path over XMP (`PhotoMetadata`) and the IIM↔XMP `IimXmpReconciler` are
in progress (issue #34).

## Status

Legacy IIM + Photoshop IRB: **implemented**. Modern IPTC-over-XMP and reconciliation: in progress.
See [STATUS.md](STATUS.md).

## License

Licensed under either of MIT or Apache-2.0 at your option.
