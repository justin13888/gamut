//! Exact-byte assertions pinning the serialization (mutation killers), plus the property-dedup
//! guarantee. These inspect raw bytes where `read` would normalise away the detail under test
//! (reserved bits, `ipma` essential flags, `ipco` sharing).

use gamut_isobmff::{
    ColourInformation, IsoBmffImage, Item, NclxColr, Property, PropertyKind, read, write,
};

fn be32(buf: &[u8], p: usize) -> u32 {
    u32::from_be_bytes([buf[p], buf[p + 1], buf[p + 2], buf[p + 3]])
}

/// Position of the first occurrence of `fourcc`.
fn find(buf: &[u8], fourcc: &[u8; 4]) -> usize {
    buf.windows(4)
        .position(|w| w == fourcc)
        .unwrap_or_else(|| panic!("box {fourcc:?} not found"))
}

/// The body bytes (after the 8-byte size+type header) of the first box of type `fourcc`.
fn box_body<'a>(buf: &'a [u8], fourcc: &[u8; 4]) -> &'a [u8] {
    let p = find(buf, fourcc);
    let size = be32(buf, p - 4) as usize;
    &buf[p + 4..p - 4 + size]
}

/// Count of non-overlapping-enough occurrences of `fourcc` (windowed; payloads avoid these codes).
fn count(buf: &[u8], fourcc: &[u8; 4]) -> usize {
    buf.windows(4).filter(|w| *w == fourcc).count()
}

fn item(id: u16, properties: Vec<Property>, payload: Vec<u8>) -> Item {
    Item {
        id,
        item_type: *b"av01",
        name: String::new(),
        properties,
        payload,
    }
}

fn file(items: Vec<Item>) -> IsoBmffImage {
    IsoBmffImage {
        major_brand: *b"avif",
        minor_version: 0,
        compatible_brands: vec![*b"avif", *b"mif1", *b"miaf", *b"MA1A"],
        primary_item_id: 1,
        items,
    }
}

fn base_props() -> Vec<Property> {
    vec![
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
    ]
}

#[test]
fn top_level_layout_is_ftyp_then_meta_then_mdat() {
    let f = write(&file(vec![item(1, base_props(), vec![1, 2, 3, 4])]));
    assert_eq!(&f[4..8], b"ftyp");
    assert!(find(&f, b"ftyp") < find(&f, b"meta"));
    assert!(find(&f, b"meta") < find(&f, b"mdat"));
}

#[test]
fn ftyp_body_is_exact() {
    let f = write(&file(vec![item(1, base_props(), vec![0xA5; 4])]));
    // major `avif`, minor 0, compatible avif/mif1/miaf/MA1A.
    assert_eq!(
        box_body(&f, b"ftyp"),
        b"avif\x00\x00\x00\x00avifmif1miafMA1A"
    );
}

#[test]
fn colr_nclx_body_encodes_every_field() {
    // Distinct non-zero CICP code points and full_range = true so every byte (incl. the bit-7
    // full_range flag) is observable.
    let props = vec![Property {
        essential: false,
        kind: PropertyKind::Colour(ColourInformation::Nclx(NclxColr {
            colour_primaries: 2,
            transfer_characteristics: 3,
            matrix_coefficients: 5,
            full_range: true,
        })),
    }];
    let f = write(&file(vec![item(1, props, vec![0; 8])]));
    assert_eq!(
        box_body(&f, b"colr"),
        &[b'n', b'c', b'l', b'x', 0, 2, 0, 3, 0, 5, 0x80]
    );
}

#[test]
fn ipma_associates_base_properties_with_essential_av1c() {
    let f = write(&file(vec![item(1, base_props(), vec![0; 8])]));
    let ipma = box_body(&f, b"ipma");
    assert_eq!(ipma[10], 4, "association_count");
    // av1C essential (0x80 | 1), then ispe/pixi/colr non-essential.
    assert_eq!(&ipma[11..15], &[0x81, 2, 3, 4]);
}

#[test]
fn ipma_marks_transforms_essential() {
    let mut props = base_props();
    props.push(Property {
        essential: true,
        kind: PropertyKind::Rotation(1),
    });
    props.push(Property {
        essential: true,
        kind: PropertyKind::Mirror(1),
    });
    let f = write(&file(vec![item(1, props, vec![0; 8])]));
    let ipma = box_body(&f, b"ipma");
    assert_eq!(ipma[10], 6, "association_count");
    assert_eq!(&ipma[11..17], &[0x81, 2, 3, 4, 0x85, 0x86]);
}

#[test]
fn iloc_extent_resolves_to_the_item_payload() {
    let payload = vec![1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    let f = write(&file(vec![item(1, base_props(), payload.clone())]));
    // The reader resolves the iloc extent against mdat; the recovered payload must be exact.
    assert_eq!(read(&f).unwrap().items[0].payload, payload);
}

#[test]
fn identical_properties_are_pooled_into_one_ipco_entry() {
    // Two items with identical ispe/pixi/colr/av1C must share one ipco entry each (dedup), not
    // duplicate them. A "never dedup" writer would emit two of each.
    let f = write(&file(vec![
        item(1, base_props(), vec![1, 1, 1]),
        item(2, base_props(), vec![2, 2, 2, 2]),
    ]));
    assert_eq!(count(&f, b"ispe"), 1, "ispe should be pooled");
    assert_eq!(count(&f, b"av1C"), 1, "av1C should be pooled");
    assert_eq!(count(&f, b"colr"), 1, "colr should be pooled");
    // ...but both items still round-trip with their full property lists.
    let parsed = read(&f).unwrap();
    assert_eq!(parsed.items.len(), 2);
    assert_eq!(parsed.items[0].properties.len(), 4);
    assert_eq!(parsed.items[1].properties.len(), 4);
}
