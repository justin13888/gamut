//! Parser robustness: malformed or out-of-scope inputs must yield a typed error, never a panic.

use gamut_isobmff::{
    ColourInformation, IsoBmffImage, Item, NclxColr, Property, PropertyKind, read, write,
};

/// A valid single-item AVIF-style file to corrupt.
fn valid() -> Vec<u8> {
    let item = Item {
        id: 1,
        item_type: *b"av01",
        name: String::new(),
        properties: vec![
            Property {
                essential: true,
                kind: PropertyKind::CodecConfiguration {
                    kind: *b"av1C",
                    data: vec![0x81, 0x20, 0x0c, 0x00],
                },
            },
            Property {
                essential: false,
                kind: PropertyKind::ImageSpatialExtents {
                    width: 16,
                    height: 16,
                },
            },
            Property {
                essential: false,
                kind: PropertyKind::Colour(ColourInformation::Nclx(NclxColr {
                    colour_primaries: 1,
                    transfer_characteristics: 13,
                    matrix_coefficients: 0,
                    full_range: true,
                })),
            },
        ],
        payload: vec![0xAB; 8],
    };
    write(&IsoBmffImage {
        major_brand: *b"avif",
        minor_version: 0,
        compatible_brands: vec![*b"avif", *b"mif1"],
        primary_item_id: 1,
        items: vec![item],
    })
}

/// Absolute position of the first occurrence of `fourcc`.
fn find(buf: &[u8], fourcc: &[u8; 4]) -> usize {
    buf.windows(4).position(|w| w == fourcc).unwrap()
}

#[test]
fn valid_file_reads_back() {
    assert!(read(&valid()).is_ok());
}

#[test]
fn empty_input_errors() {
    let e = read(&[]).unwrap_err();
    assert!(e.to_string().contains("missing ftyp"), "{e}");
}

#[test]
fn truncated_box_header_errors() {
    assert!(read(&[0, 0, 0, 8]).is_err());
}

#[test]
fn box_size_below_header_errors() {
    let e = read(&[0, 0, 0, 4, b'f', b't', b'y', b'p']).unwrap_err();
    assert!(e.to_string().contains("size smaller than header"), "{e}");
}

#[test]
fn missing_meta_errors() {
    // ftyp (no brands) + empty mdat, no meta.
    let buf = [
        0, 0, 0, 16, b'f', b't', b'y', b'p', b'a', b'v', b'i', b'f', 0, 0, 0, 0, // ftyp
        0, 0, 0, 8, b'm', b'd', b'a', b't', // mdat
    ];
    let e = read(&buf).unwrap_err();
    assert!(e.to_string().contains("missing meta"), "{e}");
}

#[test]
fn missing_mdat_errors() {
    let mut f = valid();
    let p = find(&f, b"mdat");
    f[p..p + 4].copy_from_slice(b"free"); // rename mdat → an ignored box
    let e = read(&f).unwrap_err();
    assert!(e.to_string().contains("missing mdat"), "{e}");
}

#[test]
fn tracks_are_unsupported() {
    let e = read(&[0, 0, 0, 8, b'm', b'o', b'o', b'v']).unwrap_err();
    assert!(e.to_string().contains("sequences"), "{e}");
}

#[test]
fn largesize_is_unsupported() {
    let e = read(&[0, 0, 0, 1, b'm', b'd', b'a', b't']).unwrap_err();
    assert!(e.to_string().contains("largesize"), "{e}");
}

#[test]
fn iloc_extent_out_of_bounds_errors() {
    let mut f = valid();
    let p = find(&f, b"iloc");
    // extent_offset is at body offset 14 → absolute p + 4 + 14.
    f[p + 18..p + 22].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
    let e = read(&f).unwrap_err();
    assert!(e.to_string().contains("extent out of bounds"), "{e}");
}

#[test]
fn ipma_property_index_out_of_range_errors() {
    let mut f = valid();
    let p = find(&f, b"ipma");
    // First association byte is at body offset 11 → absolute p + 4 + 11.
    f[p + 15] = 0x7f; // index 127, far beyond the 3 properties
    let e = read(&f).unwrap_err();
    assert!(e.to_string().contains("index out of range"), "{e}");
}

#[test]
fn ipma_property_index_zero_errors() {
    let mut f = valid();
    let p = find(&f, b"ipma");
    f[p + 15] = 0x00; // index 0 is invalid (1-based)
    let e = read(&f).unwrap_err();
    assert!(e.to_string().contains("index out of range"), "{e}");
}
