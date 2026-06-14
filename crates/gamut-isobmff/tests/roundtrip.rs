//! `read(&write(&img)) == img` across the shapes this crate is meant to model.
//!
//! Roundtrip is the crate's keystone correctness net (the analogue of `gamut-ifd`'s read→write→read
//! check): every supported box and property, the property-dedup pooling, and verbatim preservation
//! of unrecognised boxes must survive a write→read cycle unchanged.

use gamut_isobmff::{
    ColourInformation, IsoBmffImage, Item, NclxColr, Property, PropertyKind, read, write,
};

/// A colour image item carrying the canonical AVIF property set, parameterised by id and payload so
/// multi-item cases can reuse it.
fn av01_item(id: u16, payload: Vec<u8>) -> Item {
    Item {
        id,
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
                    width: 48,
                    height: 32,
                },
            },
            Property {
                essential: false,
                kind: PropertyKind::PixelInformation {
                    bits_per_channel: vec![8, 8, 8],
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
        payload,
    }
}

/// Wraps a single item in a complete AVIF-style file.
fn single(item: Item) -> IsoBmffImage {
    IsoBmffImage {
        major_brand: *b"avif",
        minor_version: 0,
        compatible_brands: vec![*b"avif", *b"mif1", *b"miaf", *b"MA1A"],
        primary_item_id: item.id,
        items: vec![item],
    }
}

#[track_caller]
fn assert_roundtrips(img: &IsoBmffImage) {
    assert_eq!(&read(&write(img)).unwrap(), img);
}

#[test]
fn minimal_still() {
    assert_roundtrips(&single(av01_item(1, vec![0xde, 0xad, 0xbe, 0xef, 1, 2, 3])));
}

#[test]
fn rotation_only() {
    let mut item = av01_item(1, vec![9; 16]);
    item.properties.push(Property {
        essential: true,
        kind: PropertyKind::Rotation(3),
    });
    assert_roundtrips(&single(item));
}

#[test]
fn mirror_each_axis() {
    for axis in [0u8, 1] {
        let mut item = av01_item(1, vec![7; 12]);
        item.properties.push(Property {
            essential: true,
            kind: PropertyKind::Mirror(axis),
        });
        assert_roundtrips(&single(item));
    }
}

#[test]
fn rotation_and_mirror() {
    let mut item = av01_item(1, vec![0; 8]);
    item.properties.push(Property {
        essential: true,
        kind: PropertyKind::Rotation(1),
    });
    item.properties.push(Property {
        essential: true,
        kind: PropertyKind::Mirror(1),
    });
    assert_roundtrips(&single(item));
}

#[test]
fn compatible_brand_order_is_preserved() {
    let mut img = single(av01_item(1, vec![1, 2, 3, 4]));
    img.compatible_brands = vec![*b"mif1", *b"avif", *b"MA1A", *b"miaf"];
    assert_roundtrips(&img);
}

#[test]
fn non_empty_item_name() {
    let mut item = av01_item(1, vec![5; 5]);
    item.name = "primary".to_string();
    assert_roundtrips(&single(item));
}

#[test]
fn unknown_property_preserved_verbatim() {
    let mut item = av01_item(1, vec![1; 10]);
    item.properties.push(Property {
        essential: false,
        kind: PropertyKind::Other {
            kind: *b"pasp",
            data: vec![0, 0, 0, 1, 0, 0, 0, 1], // hSpacing/vSpacing
        },
    });
    assert_roundtrips(&single(item));
}

#[test]
fn codec_configuration_with_non_trivial_data() {
    // An av1C carrying a non-empty configOBUs tail must survive as opaque bytes.
    let mut item = av01_item(1, vec![2; 6]);
    item.properties[0] = Property {
        essential: true,
        kind: PropertyKind::CodecConfiguration {
            kind: *b"av1C",
            data: vec![0x81, 0x05, 0x0c, 0x00, 0xaa, 0xbb, 0xcc],
        },
    };
    assert_roundtrips(&single(item));
}

#[test]
fn colr_full_range_false() {
    // The full_range flag (bit 7 of the last colr byte) must round-trip when clear, not just set.
    let mut item = av01_item(1, vec![3; 9]);
    item.properties[3] = Property {
        essential: false,
        kind: PropertyKind::Colour(ColourInformation::Nclx(NclxColr {
            colour_primaries: 9,
            transfer_characteristics: 16,
            matrix_coefficients: 9,
            full_range: false,
        })),
    };
    assert_roundtrips(&single(item));
}

#[test]
fn non_default_primary_item_id() {
    // A primary_item_id other than 1 must be preserved (the item id and pitm agree).
    let img = single(av01_item(7, vec![1, 2, 3, 4, 5]));
    assert_eq!(img.primary_item_id, 7);
    assert_roundtrips(&img);
}

#[test]
fn two_items_sharing_a_property() {
    // Two items whose ispe/pixi/colr are identical exercise the ipco dedup → ipma re-expand path.
    let img = IsoBmffImage {
        major_brand: *b"avif",
        minor_version: 0,
        compatible_brands: vec![*b"avif", *b"mif1"],
        primary_item_id: 1,
        items: vec![av01_item(1, vec![1, 2, 3]), av01_item(2, vec![4, 5, 6, 7])],
    };
    assert_roundtrips(&img);
}
