//! AVIF container wiring: `parse` runs `gamut_isobmff::walk_segments` and surfaces its
//! results faithfully.
//!
//! The walk's rules are pinned in `gamut-isobmff`'s own `tests/accounting.rs` -- one
//! implementation, one place that pins it (#436). What stays here is what is specific to this
//! crate: that its container type is wired to the shared walk, plus the format-specific cases
//! below.

mod common;

use common::{av01_item, clean_file};
use gamut_avif::{AvifContainer, SegmentKind};

/// Folds over the segments asserting: non-empty, first starts at 0, each end chains to the next
/// start (contiguous, non-overlapping), and the last ends at `len` — the every-byte invariant.
fn assert_covers(container: &AvifContainer, len: usize) {
    let segs = container.segments();
    assert!(!segs.is_empty(), "at least one segment");
    assert_eq!(segs[0].range.start, 0, "coverage starts at 0");
    for pair in segs.windows(2) {
        assert_eq!(
            pair[0].range.end, pair[1].range.start,
            "segments are contiguous and non-overlapping"
        );
    }
    assert_eq!(
        segs.last().unwrap().range.end,
        len,
        "coverage runs to end of file"
    );
    for s in segs {
        assert!(s.range.end > s.range.start, "no empty segment");
    }
}

/// The top-level box types, in order (the `SegmentKind::Box` segments).
fn box_types(container: &AvifContainer) -> Vec<[u8; 4]> {
    container.boxes().map(|(ty, _)| ty).collect()
}

/// `parse` runs the shared walk and exposes its results through every accessor.
///
/// The walk's own behaviour -- unknown boxes, alternate size headers, appended streams, trailers,
/// and the ftyp+meta tolerance rule -- is pinned once in `gamut-isobmff`'s `tests/accounting.rs`
/// since #436 made it one implementation. What is left for this crate to pin is that
/// `AvifContainer::parse` calls it and hands the results out unmodified, which is what a
/// mis-wiring would break.
#[test]
fn parse_exposes_the_shared_segment_walk() {
    let data = clean_file(1, vec![av01_item(1, vec![0xAA, 0xBB, 0xCC, 0xDD])]);
    let c = AvifContainer::parse(&data).unwrap();

    assert_covers(&c, data.len());
    assert_eq!(box_types(&c), vec![*b"ftyp", *b"meta", *b"mdat"]);
    assert!(c.appended_stream().is_none());
    assert!(c.trailer().is_none());
    // Every segment is a Box.
    assert!(
        c.segments()
            .iter()
            .all(|s| matches!(s.kind, SegmentKind::Box { .. }))
    );

    // `data()` returns the exact input buffer it was parsed from. Compare by contents *and*
    // pointer identity — it borrows `data`.
    assert_eq!(c.data(), data.as_slice());
    assert_eq!(c.data().as_ptr(), data.as_ptr());
    assert_eq!(c.data().len(), data.len());
}

#[test]
fn encoder_output_accounts_totally() {
    // The crate's own encoder output decomposes into exactly ftyp + meta + mdat with no appended
    // stream or trailer — the two halves of the crate agree on the container shape.
    use gamut_core::{Dimensions, EncodeImage, ImageRef, Rgb8};
    let rgb = vec![200u8; 4 * 4 * 3];
    let image = ImageRef::<Rgb8>::new(
        &rgb,
        Dimensions {
            width: 4,
            height: 4,
        },
    )
    .unwrap();
    let mut data = Vec::new();
    gamut_avif::AvifEncoder::new()
        .encode_image(image, &mut data)
        .unwrap();
    let c = AvifContainer::parse(&data).unwrap();
    assert_covers(&c, data.len());
    assert_eq!(box_types(&c), vec![*b"ftyp", *b"meta", *b"mdat"]);
    assert!(c.unknown_meta_boxes().is_empty());
    assert!(c.image().is_av1_still());
}
