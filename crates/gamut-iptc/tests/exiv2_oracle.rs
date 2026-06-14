//! Differential cross-check of gamut-iptc's legacy IIM/IRB handling against exiv2 (the reference
//! implementation), via the dev-only [`gamut_iptc_oracle`]. Covers the binary carrier; exiv2's XMP
//! toolkit is disabled in the oracle build, so the IPTC-in-XMP leg is out of scope here.
//!
//! Needs the `third_party/exiv2` submodule and a C++ toolchain + CMake/Ninja (see the oracle crate).

use gamut_iptc::{IimBlock, IimDataSet, IptcWriter};

fn ds(record: u8, dataset: u8, data: &[u8]) -> IimDataSet {
    IimDataSet {
        record,
        dataset,
        data: data.to_vec(),
    }
}

/// A spread of well-known Application-record datasets: the mandatory Record Version, single and
/// repeatable strings, and a longer caption.
fn fixture() -> IimBlock {
    IimBlock {
        datasets: vec![
            ds(2, 0, &[0, 4]),      // Record Version = 4
            ds(2, 80, b"Jane Doe"), // By-line
            ds(2, 90, b"Paris"),    // City
            ds(2, 25, b"sky"),      // Keywords (repeatable)
            ds(2, 25, b"sea"),
            ds(2, 120, b"A wide caption, with punctuation."), // Caption/Abstract
        ],
    }
}

fn multiset(block: &IimBlock) -> Vec<(u8, u8, Vec<u8>)> {
    let mut v: Vec<_> = block
        .datasets
        .iter()
        .map(|d| (d.record, d.dataset, d.data.clone()))
        .collect();
    v.sort();
    v
}

#[test]
fn gamut_iim_matches_exiv2_dataset_for_dataset() {
    let block = fixture();
    let bytes = block.encode().unwrap();

    let exiv2 = gamut_iptc_oracle::parse_iim(&bytes).expect("exiv2 parses gamut's IIM stream");
    assert_eq!(exiv2.len(), block.datasets.len());
    for (o, g) in exiv2.iter().zip(&block.datasets) {
        assert_eq!(o.record, u16::from(g.record), "record mismatch");
        assert_eq!(o.tag, u16::from(g.dataset), "dataset mismatch");
        assert_eq!(
            o.value, g.data,
            "value mismatch for {}:{}",
            g.record, g.dataset
        );
    }

    // gamut re-parses its own output identically, alongside the oracle.
    assert_eq!(IimBlock::parse(&bytes).unwrap(), block);
}

#[test]
fn gamut_reads_exiv2_reencoded_stream() {
    let block = fixture();
    let bytes = block.encode().unwrap();

    let exiv2_bytes = gamut_iptc_oracle::reencode_iim(&bytes).expect("exiv2 re-encodes the stream");
    let reparsed = IimBlock::parse(&exiv2_bytes).expect("gamut parses exiv2's output");

    // exiv2 may reorder datasets on encode; compare as multisets of (record, dataset, value).
    assert_eq!(multiset(&reparsed), multiset(&block));
}

#[test]
fn gamut_irb_payload_matches_exiv2_locate() {
    let block = fixture();
    let irb = IptcWriter::new().write_irb(&block).unwrap();

    let payload = gamut_iptc_oracle::locate_iptc_irb(&irb).expect("exiv2 locates the 0x0404 IRB");
    assert_eq!(payload, block.encode().unwrap());
}

#[test]
fn exiv2_rejects_garbage_but_accepts_gamut_output() {
    // Not an IIM stream (no 0x1C marker) and not an 8BIM resource.
    assert!(
        gamut_iptc_oracle::parse_iim(&[0xDE, 0xAD, 0xBE, 0xEF])
            .unwrap_or_default()
            .is_empty()
    );
    assert!(gamut_iptc_oracle::locate_iptc_irb(b"not a photoshop irb").is_none());
    // ...but it accepts what gamut produces.
    let bytes = fixture().encode().unwrap();
    assert!(gamut_iptc_oracle::parse_iim(&bytes).is_some());
}
