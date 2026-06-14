//! Serialises an [`IsoBmffImage`] into a single-still-image ISOBMFF file.
//!
//! The layout is `ftyp` + `meta` + `mdat`. The one keystone is the back-patch: each item's `iloc`
//! `extent_offset` can only be filled once the `mdat` payload positions are known, so the writer
//! reserves those 4-byte slots while emitting `meta` and patches them after `mdat` is placed (the
//! analogue of `gamut-ifd`'s two-pass offset layout). Box byte layouts follow ISO/IEC 14496-12
//! (ISOBMFF) and ISO/IEC 23008-12 (HEIF); see `references/isobmff`.

use crate::boxes::BoxBuilder;
use crate::model::{ColourInformation, IsoBmffImage, Item, PropertyKind};

/// Serialises `image` into a complete ISOBMFF file (`ftyp` + `meta` + `mdat`).
///
/// Offsets and lengths are written as 32-bit fields, so the file (and each item payload) must be
/// below 4 GiB — always true for a still image. [`read`](crate::read)`(&write(&image))` reproduces
/// `image` for any value this crate can construct.
#[must_use]
pub fn write(image: &IsoBmffImage) -> Vec<u8> {
    let mut bb = BoxBuilder::new();
    write_ftyp(&mut bb, image);
    let extent_slots = write_meta(&mut bb, image);

    let mdat_start = bb.begin_box(b"mdat");
    let mut payload_positions = Vec::with_capacity(image.items.len());
    for item in &image.items {
        payload_positions.push(bb.len());
        bb.bytes(&item.payload);
    }
    bb.end_box(mdat_start);

    for (slot, pos) in extent_slots.into_iter().zip(payload_positions) {
        bb.patch_u32(slot, pos as u32);
    }
    bb.into_vec()
}

/// `ftyp`: major brand, minor version, and the compatible-brand list.
fn write_ftyp(bb: &mut BoxBuilder, image: &IsoBmffImage) {
    let start = bb.begin_box(b"ftyp");
    bb.bytes(&image.major_brand);
    bb.u32(image.minor_version);
    for brand in &image.compatible_brands {
        bb.bytes(brand);
    }
    bb.end_box(start);
}

/// `meta` (FullBox v0) and its children; returns each item's reserved `iloc` `extent_offset` slot
/// in item order.
fn write_meta(bb: &mut BoxBuilder, image: &IsoBmffImage) -> Vec<usize> {
    let start = bb.begin_box(b"meta");
    bb.full_box(0, 0);
    write_hdlr(bb);
    write_pitm(bb, image.primary_item_id);
    let extent_slots = write_iloc(bb, &image.items);
    write_iinf(bb, &image.items);
    write_iprp(bb, &image.items);
    bb.end_box(start);
    extent_slots
}

/// `hdlr`: handler_type `pict` (HEIF image-item handler).
fn write_hdlr(bb: &mut BoxBuilder) {
    let start = bb.begin_box(b"hdlr");
    bb.full_box(0, 0);
    bb.u32(0); // pre_defined
    bb.bytes(b"pict"); // handler_type
    bb.u32(0); // reserved[0]
    bb.u32(0); // reserved[1]
    bb.u32(0); // reserved[2]
    bb.u8(0); // name: empty, null-terminated
    bb.end_box(start);
}

/// `pitm` v0: the primary item id.
fn write_pitm(bb: &mut BoxBuilder, primary_item_id: u16) {
    let start = bb.begin_box(b"pitm");
    bb.full_box(0, 0);
    bb.u16(primary_item_id);
    bb.end_box(start);
}

/// `iloc` v0: one extent per item, `construction_method` 0 (file offset). Reserves and returns the
/// per-item 4-byte `extent_offset` slots (patched once `mdat` is placed).
fn write_iloc(bb: &mut BoxBuilder, items: &[Item]) -> Vec<usize> {
    let start = bb.begin_box(b"iloc");
    bb.full_box(0, 0);
    bb.u8(0x44); // offset_size = 4, length_size = 4
    bb.u8(0x00); // base_offset_size = 0, reserved = 0
    bb.u16(items.len() as u16); // item_count
    let mut slots = Vec::with_capacity(items.len());
    for item in items {
        bb.u16(item.id); // item_ID
        bb.u16(0); // data_reference_index (0 = this file)
        // base_offset: 0 bytes (base_offset_size == 0)
        bb.u16(1); // extent_count
        slots.push(bb.reserve_u32()); // extent_offset (patched after mdat is placed)
        bb.u32(item.payload.len() as u32); // extent_length
    }
    bb.end_box(start);
    slots
}

/// `iinf` v0 + one `infe` v2 per item.
fn write_iinf(bb: &mut BoxBuilder, items: &[Item]) {
    let start = bb.begin_box(b"iinf");
    bb.full_box(0, 0);
    bb.u16(items.len() as u16); // entry_count
    for item in items {
        let infe = bb.begin_box(b"infe");
        bb.full_box(2, 0); // version 2, flags 0 (visible item)
        bb.u16(item.id); // item_ID
        bb.u16(0); // item_protection_index
        bb.bytes(&item.item_type); // item_type
        bb.bytes(item.name.as_bytes()); // item_name
        bb.u8(0); // item_name null terminator
        bb.end_box(infe);
    }
    bb.end_box(start);
}

/// `iprp` = a shared `ipco` (deduplicated property boxes) + `ipma` associating them with each item.
fn write_iprp(bb: &mut BoxBuilder, items: &[Item]) {
    // Build the shared ipco pool, deduplicating by serialized bytes. The essential flag is an ipma
    // concern (it is not part of the property box), so two items may share a property at different
    // essentiality. `assoc[i]` is item i's associations as `(1-based pool index, essential)`.
    let mut pool: Vec<Vec<u8>> = Vec::new();
    let mut assoc: Vec<Vec<(usize, bool)>> = Vec::with_capacity(items.len());
    for item in items {
        let mut row = Vec::with_capacity(item.properties.len());
        for property in &item.properties {
            let bytes = serialize_property(&property.kind);
            let index = match pool.iter().position(|p| *p == bytes) {
                Some(i) => i + 1,
                None => {
                    pool.push(bytes);
                    pool.len()
                }
            };
            row.push((index, property.essential));
        }
        assoc.push(row);
    }

    let start = bb.begin_box(b"iprp");
    let ipco = bb.begin_box(b"ipco");
    for property in &pool {
        bb.bytes(property);
    }
    bb.end_box(ipco);
    write_ipma(bb, items, &assoc);
    bb.end_box(start);
}

/// `ipma` v0: each item id → its `(property_index, essential)` associations, in association order.
///
/// `flags = 0` makes each association a single byte `essential(1) | index(7)`; this crate only emits
/// the v0 single-byte form, which holds while every property index ≤ 127 (HEIF still images use a
/// handful of properties, so this always holds).
fn write_ipma(bb: &mut BoxBuilder, items: &[Item], assoc: &[Vec<(usize, bool)>]) {
    let start = bb.begin_box(b"ipma");
    bb.full_box(0, 0);
    bb.u32(items.len() as u32); // entry_count
    for (item, row) in items.iter().zip(assoc) {
        bb.u16(item.id);
        debug_assert!(row.len() <= usize::from(u8::MAX));
        bb.u8(row.len() as u8); // association_count
        for &(index, essential) in row {
            debug_assert!(index <= 0x7f, "ipma v0 holds at most 127 properties");
            // The essential flag is bit 7; the property index (≤ 127) occupies bits 0..6. Written as
            // an addition rather than `0x80 | index` so the operator is mutation-observable (OR/XOR/
            // ADD all coincide for the disjoint bit 7, which would otherwise leave an equivalent
            // mutant).
            let byte = index as u8;
            bb.u8(if essential { byte + 0x80 } else { byte });
        }
    }
    bb.end_box(start);
}

/// Serialises one property as a complete box (size + type + body). The `essential` flag is *not*
/// encoded here — it lives in `ipma`.
fn serialize_property(kind: &PropertyKind) -> Vec<u8> {
    let mut bb = BoxBuilder::new();
    match kind {
        PropertyKind::ImageSpatialExtents { width, height } => {
            let start = bb.begin_box(b"ispe");
            bb.full_box(0, 0);
            bb.u32(*width);
            bb.u32(*height);
            bb.end_box(start);
        }
        PropertyKind::PixelInformation { bits_per_channel } => {
            let start = bb.begin_box(b"pixi");
            bb.full_box(0, 0);
            debug_assert!(bits_per_channel.len() <= usize::from(u8::MAX));
            bb.u8(bits_per_channel.len() as u8);
            for &bits in bits_per_channel {
                bb.u8(bits);
            }
            bb.end_box(start);
        }
        PropertyKind::Colour(ColourInformation::Nclx(c)) => {
            let start = bb.begin_box(b"colr");
            bb.bytes(b"nclx");
            bb.u16(c.colour_primaries);
            bb.u16(c.transfer_characteristics);
            bb.u16(c.matrix_coefficients);
            bb.u8(u8::from(c.full_range) << 7); // full_range_flag in bit 7, reserved = 0
            bb.end_box(start);
        }
        PropertyKind::Rotation(angle) => {
            let start = bb.begin_box(b"irot");
            bb.u8(angle & 0x03); // reserved(6) | angle(2)
            bb.end_box(start);
        }
        PropertyKind::Mirror(axis) => {
            let start = bb.begin_box(b"imir");
            bb.u8(axis & 0x01); // reserved(7) | axis(1)
            bb.end_box(start);
        }
        PropertyKind::CodecConfiguration { kind, data } | PropertyKind::Other { kind, data } => {
            let start = bb.begin_box(kind);
            bb.bytes(data);
            bb.end_box(start);
        }
    }
    bb.into_vec()
}
