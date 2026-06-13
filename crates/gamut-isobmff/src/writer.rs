//! Assembles a single-still-image AVIF file (AVIF v1.2.0 §9.1.1 minimum box set).

use crate::boxes::BoxBuilder;

/// The AV1 configuration record stamped into the `av1C` property. Every field **must** equal the
/// corresponding value in the AV1 sequence header (AV1-ISOBMFF v1.3.0 §2.3.4); the caller obtains
/// them from the encoded sequence header so they match by construction.
#[derive(Debug, Clone, Copy)]
pub struct Av1cConfig {
    /// `seq_profile` (3 bits).
    pub seq_profile: u8,
    /// `seq_level_idx[0]` (5 bits).
    pub seq_level_idx_0: u8,
    /// `seq_tier[0]` (1 bit).
    pub seq_tier_0: u8,
    /// `high_bitdepth` flag (1 bit).
    pub high_bitdepth: bool,
    /// `twelve_bit` flag (1 bit).
    pub twelve_bit: bool,
    /// `mono_chrome` flag (1 bit).
    pub monochrome: bool,
    /// `subsampling_x` (1 bit).
    pub chroma_subsampling_x: u8,
    /// `subsampling_y` (1 bit).
    pub chroma_subsampling_y: u8,
    /// `chroma_sample_position` (2 bits).
    pub chroma_sample_position: u8,
}

/// The nclx colour information written into the `colr` box (CICP code points). `matrix_coefficients`
/// and `full_range` **must** match the AV1 sequence header (AV1-ISOBMFF v1.3.0 §2.3.4).
#[derive(Debug, Clone, Copy)]
pub struct NclxColr {
    /// CICP colour primaries.
    pub colour_primaries: u16,
    /// CICP transfer characteristics.
    pub transfer_characteristics: u16,
    /// CICP matrix coefficients.
    pub matrix_coefficients: u16,
    /// Full-range flag.
    pub full_range: bool,
}

/// Image-orientation transform properties (`irot`/`imir`, ISO/IEC 23008-12 §6.5.10/§6.5.12),
/// applied by a reader at display time — the stored pixels are unchanged, so this records e.g. an
/// EXIF orientation without re-encoding rotated samples. Both are transformative properties and are
/// therefore associated as *essential* (MIAF §7.3.6.7). They apply in the order `irot` then `imir`.
#[derive(Debug, Clone, Copy, Default)]
pub struct ImageTransform {
    /// `irot` rotation in 90° steps (`angle`, 0..=3), applied anti-clockwise per ISO/IEC 23008-12.
    /// `0` writes no `irot` box.
    pub rotation_ccw: u8,
    /// `imir` mirror axis: `Some(0)` mirrors about a vertical axis (left↔right), `Some(1)` about a
    /// horizontal axis (top↔bottom). `None` writes no `imir` box.
    pub mirror_axis: Option<u8>,
}

/// Everything needed to serialize one AVIF still image.
#[derive(Debug, Clone)]
pub struct AvifStillImage<'a> {
    /// Image width in pixels (written to `ispe`; must equal AV1 `UpscaledWidth`).
    pub width: u32,
    /// Image height in pixels (written to `ispe`; must equal AV1 `FrameHeight`).
    pub height: u32,
    /// Bits per channel (written to `pixi`; must match `av1C` bit depth).
    pub bit_depth: u8,
    /// Number of channels (written to `pixi`; 3 for colour, must match `av1C` `mono_chrome`).
    pub num_channels: u8,
    /// AV1 configuration record for `av1C`.
    pub av1c: Av1cConfig,
    /// nclx colour information for `colr`.
    pub nclx: NclxColr,
    /// Optional `irot`/`imir` display-orientation transforms (default: none).
    pub transform: ImageTransform,
    /// The AV1 temporal unit (sequence header OBU + frame OBU) placed in `mdat`.
    pub item_data: &'a [u8],
}

/// Serializes `img` into a complete AVIF file (`ftyp` + `meta` + `mdat`), back-patching the `iloc`
/// extent offset to point at the `mdat` payload.
///
/// Offsets and lengths are written as 32-bit fields, so `item_data` and the file must each be below
/// 4 GiB — always true for a single still image.
#[must_use]
pub fn write_avif_still(img: &AvifStillImage) -> Vec<u8> {
    let mut bb = BoxBuilder::new();
    write_ftyp(&mut bb);
    let extent_offset_pos = write_meta(&mut bb, img);

    let mdat_start = bb.begin_box(b"mdat");
    let payload_pos = bb.len();
    bb.bytes(img.item_data);
    bb.end_box(mdat_start);

    bb.patch_u32(extent_offset_pos, payload_pos as u32);
    bb.into_vec()
}

/// `ftyp`: major brand `avif`, compatible `avif`/`mif1`/`miaf`/`MA1A` (AVIF §6, §8.3 Advanced).
fn write_ftyp(bb: &mut BoxBuilder) {
    let start = bb.begin_box(b"ftyp");
    bb.bytes(b"avif"); // major_brand
    bb.u32(0); // minor_version
    bb.bytes(b"avif");
    bb.bytes(b"mif1");
    bb.bytes(b"miaf");
    bb.bytes(b"MA1A");
    bb.end_box(start);
}

/// `meta` and all of its children; returns the reserved position of the `iloc` extent offset.
fn write_meta(bb: &mut BoxBuilder, img: &AvifStillImage) -> usize {
    let start = bb.begin_box(b"meta");
    bb.full_box(0, 0);
    write_hdlr(bb);
    write_pitm(bb);
    let extent_offset_pos = write_iloc(bb, img.item_data.len() as u32);
    write_iinf(bb);
    write_iprp(bb, img);
    bb.end_box(start);
    extent_offset_pos
}

/// `hdlr`: handler_type `pict` (HEIF image item handler).
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

/// `pitm`: primary item id = 1.
fn write_pitm(bb: &mut BoxBuilder) {
    let start = bb.begin_box(b"pitm");
    bb.full_box(0, 0);
    bb.u16(1); // item_ID
    bb.end_box(start);
}

/// `iloc` v0: one item, one extent, `construction_method` 0 (file offset). Reserves and returns the
/// 4-byte `extent_offset` slot.
fn write_iloc(bb: &mut BoxBuilder, extent_length: u32) -> usize {
    let start = bb.begin_box(b"iloc");
    bb.full_box(0, 0);
    bb.u8(0x44); // offset_size = 4, length_size = 4
    bb.u8(0x00); // base_offset_size = 0, reserved = 0
    bb.u16(1); // item_count
    bb.u16(1); // item_ID
    bb.u16(0); // data_reference_index (0 = this file)
    // base_offset: 0 bytes (base_offset_size == 0)
    bb.u16(1); // extent_count
    let extent_offset_pos = bb.reserve_u32(); // extent_offset (patched after mdat is placed)
    bb.u32(extent_length); // extent_length
    bb.end_box(start);
    extent_offset_pos
}

/// `iinf` + `infe` v2 for the single `av01` item.
fn write_iinf(bb: &mut BoxBuilder) {
    let start = bb.begin_box(b"iinf");
    bb.full_box(0, 0);
    bb.u16(1); // entry_count
    let infe = bb.begin_box(b"infe");
    bb.full_box(2, 0); // version 2, flags 0 (visible item)
    bb.u16(1); // item_ID
    bb.u16(0); // item_protection_index
    bb.bytes(b"av01"); // item_type
    bb.u8(0); // item_name: empty, null-terminated
    bb.end_box(infe);
    bb.end_box(start);
}

/// `iprp` = `ipco` (av1C, ispe, pixi, colr, optional irot/imir) + `ipma` associating them with
/// item 1. Association entries are `(property_index, essential)`: `av1C` and the transformative
/// properties are essential; `ispe`/`pixi`/`colr` are not.
fn write_iprp(bb: &mut BoxBuilder, img: &AvifStillImage) {
    let start = bb.begin_box(b"iprp");
    let ipco = bb.begin_box(b"ipco");
    write_av1c(bb, &img.av1c); // property index 1
    write_ispe(bb, img.width, img.height); // 2
    write_pixi(bb, img.num_channels, img.bit_depth); // 3
    write_colr(bb, &img.nclx); // 4
    let mut assoc = vec![(1u8, true), (2, false), (3, false), (4, false)];
    let mut next_index = 5u8;
    if img.transform.rotation_ccw != 0 {
        write_irot(bb, img.transform.rotation_ccw);
        assoc.push((next_index, true));
        next_index += 1;
    }
    if let Some(axis) = img.transform.mirror_axis {
        write_imir(bb, axis);
        assoc.push((next_index, true));
    }
    bb.end_box(ipco);
    write_ipma(bb, &assoc);
    bb.end_box(start);
}

/// `irot`: image rotation property (ISO/IEC 23008-12 §6.5.10). A plain `Box` (not a `FullBox`) whose
/// single byte is `reserved(6) | angle(2)`; `angle` is the anti-clockwise rotation in 90° steps.
fn write_irot(bb: &mut BoxBuilder, angle: u8) {
    let start = bb.begin_box(b"irot");
    bb.u8(angle & 0x03);
    bb.end_box(start);
}

/// `imir`: image mirroring property (ISO/IEC 23008-12 §6.5.12). A plain `Box` whose single byte is
/// `reserved(7) | axis(1)`; `axis` 0 mirrors about a vertical axis, 1 about a horizontal axis.
fn write_imir(bb: &mut BoxBuilder, axis: u8) {
    let start = bb.begin_box(b"imir");
    bb.u8(axis & 0x01);
    bb.end_box(start);
}

/// `av1C`: the 4-byte `AV1CodecConfigurationRecord`, empty `configOBUs` (AV1-ISOBMFF v1.3.0 §2.3.3).
fn write_av1c(bb: &mut BoxBuilder, c: &Av1cConfig) {
    let start = bb.begin_box(b"av1C");
    bb.u8(0x81); // marker = 1, version = 1
    // Each sub-field occupies a disjoint bit range, so `+` equals `|` here; it is written as `+`
    // (matching `write_ipma`) so a mutated operator changes the byte rather than leaving an
    // equivalent OR/XOR mutant. `av1c_and_colr_bodies_encode_every_field` pins every field.
    bb.u8((c.seq_profile << 5) + (c.seq_level_idx_0 & 0x1f));
    bb.u8((c.seq_tier_0 << 7)
        + (u8::from(c.high_bitdepth) << 6)
        + (u8::from(c.twelve_bit) << 5)
        + (u8::from(c.monochrome) << 4)
        + (c.chroma_subsampling_x << 3)
        + (c.chroma_subsampling_y << 2)
        + (c.chroma_sample_position & 0x3));
    bb.u8(0x00); // reserved(3)=0, initial_presentation_delay_present(1)=0, reserved(4)=0
    // configOBUs: empty (sequence header lives only in the sample)
    bb.end_box(start);
}

/// `ispe`: image spatial extents (HEIF).
fn write_ispe(bb: &mut BoxBuilder, width: u32, height: u32) {
    let start = bb.begin_box(b"ispe");
    bb.full_box(0, 0);
    bb.u32(width);
    bb.u32(height);
    bb.end_box(start);
}

/// `pixi`: pixel information — channel count and bits per channel (HEIF).
fn write_pixi(bb: &mut BoxBuilder, num_channels: u8, bit_depth: u8) {
    let start = bb.begin_box(b"pixi");
    bb.full_box(0, 0);
    bb.u8(num_channels);
    for _ in 0..num_channels {
        bb.u8(bit_depth);
    }
    bb.end_box(start);
}

/// `colr` with `colour_type` `nclx` (ISOBMFF ColourInformationBox).
fn write_colr(bb: &mut BoxBuilder, c: &NclxColr) {
    let start = bb.begin_box(b"colr");
    bb.bytes(b"nclx");
    bb.u16(c.colour_primaries);
    bb.u16(c.transfer_characteristics);
    bb.u16(c.matrix_coefficients);
    bb.u8(u8::from(c.full_range) << 7); // full_range_flag in bit 7, reserved = 0
    bb.end_box(start);
}

/// `ipma` v0: item 1 → its properties. `assoc` lists `(property_index, essential)` in association
/// order; with `flags = 0` each association is a single byte `essential(1) | index(7)` (indices stay
/// ≤ 127, so 7 bits suffice).
fn write_ipma(bb: &mut BoxBuilder, assoc: &[(u8, bool)]) {
    let start = bb.begin_box(b"ipma");
    bb.full_box(0, 0);
    bb.u32(1); // entry_count
    bb.u16(1); // item_ID
    bb.u8(assoc.len() as u8); // association_count
    for &(index, essential) in assoc {
        // The essential flag is bit 7; the property index (≤ 127) occupies bits 0..6. Written as an
        // addition rather than `0x80 | index` so the operator is mutation-observable (OR/XOR/ADD all
        // coincide for the disjoint bit 7, which would otherwise leave an equivalent mutant).
        bb.u8(if essential { index + 0x80 } else { index });
    }
    bb.end_box(start);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads a big-endian u32 at `pos`.
    fn be32(buf: &[u8], pos: usize) -> u32 {
        u32::from_be_bytes([buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]])
    }

    /// Walks top-level boxes, returning `(type, body_start, body_len)` for each.
    fn top_level_boxes(buf: &[u8]) -> Vec<([u8; 4], usize, usize)> {
        let mut out = Vec::new();
        let mut pos = 0;
        while pos + 8 <= buf.len() {
            let size = be32(buf, pos) as usize;
            let ty = [buf[pos + 4], buf[pos + 5], buf[pos + 6], buf[pos + 7]];
            assert!(
                size >= 8 && pos + size <= buf.len(),
                "bad box size {size} at {pos}"
            );
            out.push((ty, pos + 8, size - 8));
            pos += size;
        }
        assert_eq!(pos, buf.len(), "boxes do not tile the file exactly");
        out
    }

    fn sample_image(item: &[u8]) -> Vec<u8> {
        let img = AvifStillImage {
            width: 4,
            height: 4,
            bit_depth: 8,
            num_channels: 3,
            av1c: Av1cConfig {
                seq_profile: 1,
                seq_level_idx_0: 1,
                seq_tier_0: 0,
                high_bitdepth: false,
                twelve_bit: false,
                monochrome: false,
                chroma_subsampling_x: 0,
                chroma_subsampling_y: 0,
                chroma_sample_position: 0,
            },
            nclx: NclxColr {
                colour_primaries: 1,
                transfer_characteristics: 13,
                matrix_coefficients: 0,
                full_range: true,
            },
            transform: ImageTransform::default(),
            item_data: item,
        };
        write_avif_still(&img)
    }

    #[test]
    fn top_level_layout_is_ftyp_meta_mdat() {
        let item = [0xde, 0xad, 0xbe, 0xef, 0x01, 0x02];
        let file = sample_image(&item);
        let boxes = top_level_boxes(&file);
        let types: Vec<[u8; 4]> = boxes.iter().map(|b| b.0).collect();
        assert_eq!(types, vec![*b"ftyp", *b"meta", *b"mdat"]);
    }

    #[test]
    fn ftyp_lists_required_brands() {
        let file = sample_image(&[0u8; 4]);
        let (_, body, len) = top_level_boxes(&file)[0];
        let ftyp = &file[body..body + len];
        assert_eq!(&ftyp[0..4], b"avif"); // major
        let rest = &ftyp[8..]; // skip major + minor_version
        for brand in [b"avif", b"mif1", b"miaf", b"MA1A"] {
            assert!(rest.windows(4).any(|w| w == brand), "missing brand");
        }
    }

    #[test]
    fn iloc_extent_points_at_mdat_payload() {
        let item = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let file = sample_image(&item);
        let boxes = top_level_boxes(&file);
        // mdat payload = its body (8-byte header already stripped by top_level_boxes).
        let (_, mdat_body, mdat_len) = *boxes.iter().find(|b| &b.0 == b"mdat").unwrap();
        assert_eq!(&file[mdat_body..mdat_body + mdat_len], &item);

        // Find the iloc extent_offset/length by scanning the file for the 'iloc' box.
        let iloc_pos = file.windows(4).position(|w| w == b"iloc").unwrap();
        // iloc body starts after the 4cc; layout: fullbox(4) + 0x44 + 0x00 + item_count(2)
        //   + item_ID(2) + data_ref(2) + extent_count(2) + extent_offset(4) + extent_length(4).
        let body = iloc_pos + 4;
        let extent_offset = be32(&file, body + 4 + 1 + 1 + 2 + 2 + 2 + 2);
        let extent_length = be32(&file, body + 4 + 1 + 1 + 2 + 2 + 2 + 2 + 4);
        assert_eq!(
            extent_offset as usize, mdat_body,
            "extent offset must hit mdat payload"
        );
        assert_eq!(extent_length as usize, item.len());
        assert_eq!(
            &file[extent_offset as usize..(extent_offset + extent_length) as usize],
            &item
        );
    }

    #[test]
    fn meta_contains_required_property_boxes() {
        let file = sample_image(&[0u8; 8]);
        for fourcc in [
            b"hdlr", b"pitm", b"iinf", b"infe", b"iprp", b"ipco", b"ipma", b"av1C", b"ispe",
            b"pixi", b"colr",
        ] {
            assert!(
                file.windows(4).any(|w| w == fourcc),
                "missing box {fourcc:?}"
            );
        }
    }

    /// Returns the body bytes (after the 8-byte size+type header) of the first box of type `fourcc`.
    fn box_body<'a>(buf: &'a [u8], fourcc: &[u8; 4]) -> &'a [u8] {
        let p = buf
            .windows(4)
            .position(|w| w == fourcc)
            .unwrap_or_else(|| panic!("box {fourcc:?} not found"));
        let size = be32(buf, p - 4) as usize;
        &buf[p + 4..p - 4 + size]
    }

    fn image_with_transform(item: &[u8], transform: ImageTransform) -> Vec<u8> {
        let img = AvifStillImage {
            width: 4,
            height: 4,
            bit_depth: 8,
            num_channels: 3,
            av1c: Av1cConfig {
                seq_profile: 1,
                seq_level_idx_0: 1,
                seq_tier_0: 0,
                high_bitdepth: false,
                twelve_bit: false,
                monochrome: false,
                chroma_subsampling_x: 0,
                chroma_subsampling_y: 0,
                chroma_sample_position: 0,
            },
            nclx: NclxColr {
                colour_primaries: 1,
                transfer_characteristics: 13,
                matrix_coefficients: 0,
                full_range: true,
            },
            transform,
            item_data: item,
        };
        write_avif_still(&img)
    }

    #[test]
    fn no_transform_writes_no_irot_or_imir() {
        let file = image_with_transform(&[0u8; 8], ImageTransform::default());
        assert!(
            !file.windows(4).any(|w| w == b"irot"),
            "irot must be absent"
        );
        assert!(
            !file.windows(4).any(|w| w == b"imir"),
            "imir must be absent"
        );
        // ipma still associates exactly the four base properties.
        let ipma = box_body(&file, b"ipma");
        assert_eq!(ipma[4 + 4 + 2], 4, "association_count");
        assert_eq!(
            &ipma[4 + 4 + 2 + 1..4 + 4 + 2 + 1 + 4],
            &[0x80 | 1, 2, 3, 4]
        );
    }

    #[test]
    fn irot_and_imir_are_written_essential() {
        // 90° anti-clockwise rotation + a horizontal-axis mirror.
        let file = image_with_transform(
            &[0u8; 8],
            ImageTransform {
                rotation_ccw: 1,
                mirror_axis: Some(1),
            },
        );
        // irot/imir are plain Boxes with a single byte: reserved + angle/axis.
        let irot = box_body(&file, b"irot");
        assert_eq!(irot, &[1], "irot angle = 1, reserved bits zero");
        let imir = box_body(&file, b"imir");
        assert_eq!(imir, &[1], "imir axis = 1, reserved bits zero");
        // ipma associates the four base properties plus irot (index 5) and imir (index 6); both
        // transformative properties are essential (high bit set).
        let ipma = box_body(&file, b"ipma");
        assert_eq!(ipma[4 + 4 + 2], 6, "association_count");
        assert_eq!(
            &ipma[4 + 4 + 2 + 1..4 + 4 + 2 + 1 + 6],
            &[0x80 | 1, 2, 3, 4, 0x80 | 5, 0x80 | 6]
        );
    }

    #[test]
    fn rotation_only_uses_index_five() {
        // With no mirror, a rotation takes property index 5 and is the only extra association.
        let file = image_with_transform(
            &[0u8; 8],
            ImageTransform {
                rotation_ccw: 3,
                mirror_axis: None,
            },
        );
        assert_eq!(box_body(&file, b"irot"), &[3]);
        assert!(!file.windows(4).any(|w| w == b"imir"));
        let ipma = box_body(&file, b"ipma");
        assert_eq!(ipma[4 + 4 + 2], 5);
        assert_eq!(
            &ipma[4 + 4 + 2 + 1..4 + 4 + 2 + 1 + 5],
            &[0x80 | 1, 2, 3, 4, 0x80 | 5]
        );
    }

    #[test]
    fn mirror_only_axis_zero_uses_index_five() {
        // A vertical-axis mirror with no rotation: `imir` body is the axis byte 0 (distinct from the
        // axis-1 case above), and it takes property index 5.
        let file = image_with_transform(
            &[0u8; 8],
            ImageTransform {
                rotation_ccw: 0,
                mirror_axis: Some(0),
            },
        );
        assert_eq!(box_body(&file, b"imir"), &[0]);
        assert!(!file.windows(4).any(|w| w == b"irot"));
        let ipma = box_body(&file, b"ipma");
        assert_eq!(ipma[4 + 4 + 2], 5);
        assert_eq!(
            &ipma[4 + 4 + 2 + 1..4 + 4 + 2 + 1 + 5],
            &[0x80 | 1, 2, 3, 4, 0x80 | 5]
        );
    }

    #[test]
    fn av1c_and_colr_bodies_encode_every_field() {
        // Distinct, non-zero values in every av1C/colr field so each shift, mask, and combine is
        // observable (the sample image leaves most of these fields zero, hiding the encoding).
        let img = AvifStillImage {
            width: 4,
            height: 4,
            bit_depth: 8,
            num_channels: 3,
            av1c: Av1cConfig {
                // Every field non-zero so each `+`/`<<` is observable — a zero field would make its
                // term `0`, leaving the operator unobservable (`0 + x == 0 - x`, `0 << n == 0 >> n`).
                seq_profile: 5,        // 0b101
                seq_level_idx_0: 0x15, // 0b10101
                seq_tier_0: 1,
                high_bitdepth: true,
                twelve_bit: true,
                monochrome: true,
                chroma_subsampling_x: 1,
                chroma_subsampling_y: 1,
                chroma_sample_position: 2, // 0b10
            },
            nclx: NclxColr {
                colour_primaries: 2,
                transfer_characteristics: 3,
                matrix_coefficients: 5,
                full_range: true,
            },
            transform: ImageTransform::default(),
            item_data: &[0u8; 8],
        };
        let file = write_avif_still(&img);
        // marker/version 0x81; (seq_profile<<5)+(level&0x1f)=0xA0+0x15=0xB5; the flags byte sets
        // tier/high_bitdepth/twelve_bit/monochrome/subsampling_x/_y plus chroma position 2:
        // 0x80+0x40+0x20+0x10+0x08+0x04+0x02=0xFE; trailing reserved 0x00.
        assert_eq!(box_body(&file, b"av1C"), &[0x81, 0xB5, 0xFE, 0x00]);
        // 'nclx' + big-endian u16 primaries/transfer/matrix + full_range_flag in bit 7.
        assert_eq!(
            box_body(&file, b"colr"),
            &[b'n', b'c', b'l', b'x', 0, 2, 0, 3, 0, 5, 0x80]
        );
    }
}
