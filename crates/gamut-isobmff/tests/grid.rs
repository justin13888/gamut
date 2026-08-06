//! `ImageGrid` payload parse/serialise round-trip plus hand-authored layout fixtures
//! (ISO/IEC 23008-12 §6.6.2.3.2), pinned independently of the serialiser.

use gamut_core::ErrorKind;
use gamut_isobmff::ImageGrid;

#[track_caller]
fn assert_roundtrips(g: ImageGrid) {
    let bytes = g.to_bytes().unwrap();
    assert_eq!(ImageGrid::parse(&bytes).unwrap(), g);
}

#[test]
fn roundtrips_16bit_and_32bit_forms() {
    // Output dims within u16 → the compact 8-byte form.
    let small = ImageGrid {
        rows: 2,
        columns: 3,
        output_width: 4096,
        output_height: 2160,
    };
    assert_eq!(small.to_bytes().unwrap().len(), 8);
    assert_roundtrips(small);

    // A dim past u16 → the 12-byte form.
    let big = ImageGrid {
        rows: 1,
        columns: 1,
        output_width: 70_000,
        output_height: 3,
    };
    assert_eq!(big.to_bytes().unwrap().len(), 12);
    assert_roundtrips(big);
}

#[test]
fn tile_count_boundaries_roundtrip() {
    for &(rows, columns) in &[(1, 1), (1, 256), (256, 1), (256, 256)] {
        assert_roundtrips(ImageGrid {
            rows,
            columns,
            output_width: 640,
            output_height: 480,
        });
    }
}

#[test]
fn output_dimension_form_switches_at_u16_max() {
    let at = ImageGrid {
        rows: 4,
        columns: 4,
        output_width: u32::from(u16::MAX),
        output_height: u32::from(u16::MAX),
    };
    assert_eq!(
        at.to_bytes().unwrap().len(),
        8,
        "exactly u16::MAX stays 16-bit"
    );
    let over = ImageGrid {
        rows: 4,
        columns: 4,
        output_width: u32::from(u16::MAX) + 1,
        output_height: 1,
    };
    assert_eq!(
        over.to_bytes().unwrap().len(),
        12,
        "one past u16::MAX goes 32-bit"
    );
    assert_roundtrips(at);
    assert_roundtrips(over);
}

#[test]
fn parses_hand_authored_16bit_layout() {
    // version=0, flags=0, rows_minus_one=1, columns_minus_one=1, output 4096×4096 (16-bit dims).
    let bytes = [0x00, 0x00, 0x01, 0x01, 0x10, 0x00, 0x10, 0x00];
    assert_eq!(
        ImageGrid::parse(&bytes).unwrap(),
        ImageGrid {
            rows: 2,
            columns: 2,
            output_width: 4096,
            output_height: 4096,
        }
    );
}

#[test]
fn parses_hand_authored_32bit_layout() {
    // version=0, flags=1 (32-bit dims), rows_minus_one=0, columns_minus_one=2, output 70000×1.
    let bytes = [
        0x00, 0x01, 0x00, 0x02, 0x00, 0x01, 0x11, 0x70, 0x00, 0x00, 0x00, 0x01,
    ];
    assert_eq!(
        ImageGrid::parse(&bytes).unwrap(),
        ImageGrid {
            rows: 1,
            columns: 3,
            output_width: 70_000,
            output_height: 1,
        }
    );
}

#[test]
fn to_bytes_matches_hand_authored_layout() {
    let g = ImageGrid {
        rows: 2,
        columns: 2,
        output_width: 4096,
        output_height: 4096,
    };
    assert_eq!(
        g.to_bytes().unwrap(),
        vec![0x00, 0x00, 0x01, 0x01, 0x10, 0x00, 0x10, 0x00]
    );
}

#[test]
fn rejects_nonzero_version() {
    let bytes = [0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00];
    assert_eq!(
        ImageGrid::parse(&bytes).unwrap_err().kind(),
        ErrorKind::Unsupported
    );
}

#[test]
fn rejects_truncated_payload() {
    // The 16-bit form needs 8 bytes; supply 7.
    let bytes = [0x00, 0x00, 0x01, 0x01, 0x10, 0x00, 0x10];
    assert_eq!(
        ImageGrid::parse(&bytes).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
}

#[test]
fn rejects_trailing_bytes() {
    // A valid 8-byte 16-bit payload with one extra byte appended.
    let bytes = [0x00, 0x00, 0x01, 0x01, 0x10, 0x00, 0x10, 0x00, 0xff];
    assert_eq!(
        ImageGrid::parse(&bytes).unwrap_err().kind(),
        ErrorKind::InvalidInput
    );
}

#[test]
fn to_bytes_rejects_out_of_range_tile_counts() {
    for g in [
        ImageGrid {
            rows: 0,
            columns: 1,
            output_width: 8,
            output_height: 8,
        },
        ImageGrid {
            rows: 257,
            columns: 1,
            output_width: 8,
            output_height: 8,
        },
        ImageGrid {
            rows: 1,
            columns: 0,
            output_width: 8,
            output_height: 8,
        },
        ImageGrid {
            rows: 1,
            columns: 257,
            output_width: 8,
            output_height: 8,
        },
    ] {
        assert_eq!(g.to_bytes().unwrap_err().kind(), ErrorKind::InvalidInput);
    }
}
