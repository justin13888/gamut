//! The pluggable [`Av1StillDecoder`] hook and the decode pipeline (issue #250).
//!
//! A `Mock` decoder derives a **deterministic position-dependent gradient** frame from the coded
//! payload (the frame OBU encodes `(base, width, height)`, the `av1C` the chroma/bit-depth), so
//! every test asserts golden-exact samples/pixels and a placement/rotation/crop error cannot hide
//! behind a solid colour. The grid tests compare against independently-written references.

use gamut_avif::{
    Av1Config, Av1StillDecoder, AvifContainer, ChromaFormat, DecodedFrame, ObuType, iter_obus,
};
use gamut_core::{Error, Result};
use gamut_isobmff::{
    ImageGrid, ImageOverlay, IsoBmffImage, Item, ItemReference, Property, PropertyKind, write,
};

// ---- deterministic mock decoder --------------------------------------------------------------

/// The luma value the mock produces at `(x, y)` for a frame keyed by `base`, masked to
/// `bit_depth`.
fn ey(base: u8, x: u32, y: u32, bd: u8) -> u16 {
    ((u32::from(base) + x * 3 + y * 17) as u16) & mask_for(bd)
}
/// The Cb value the mock produces at chroma `(x, y)`.
fn ecb(base: u8, x: u32, y: u32, bd: u8) -> u16 {
    ((u32::from(base) + 40 + x * 5 + y * 11) as u16) & mask_for(bd)
}
/// The Cr value the mock produces at chroma `(x, y)`.
fn ecr(base: u8, x: u32, y: u32, bd: u8) -> u16 {
    ((u32::from(base) + 90 + x * 7 + y * 13) as u16) & mask_for(bd)
}
fn mask_for(bd: u8) -> u16 {
    if bd >= 16 { 0xFFFF } else { (1u16 << bd) - 1 }
}

fn build_frame(base: u8, w: u32, h: u32, chroma: ChromaFormat, bd: u8) -> Result<DecodedFrame> {
    let (wu, hu) = (w as usize, h as usize);
    let y: Vec<u16> = (0..wu * hu)
        .map(|i| ey(base, (i % wu) as u32, (i / wu) as u32, bd))
        .collect();
    let (cb, cr) = if chroma == ChromaFormat::Monochrome {
        (Vec::new(), Vec::new())
    } else {
        let (cw, ch) = chroma.chroma_dimensions(w, h);
        let (cwu, chu) = (cw as usize, ch as usize);
        let cb = (0..cwu * chu)
            .map(|i| ecb(base, (i % cwu) as u32, (i / cwu) as u32, bd))
            .collect();
        let cr = (0..cwu * chu)
            .map(|i| ecr(base, (i % cwu) as u32, (i / cwu) as u32, bd))
            .collect();
        (cb, cr)
    };
    DecodedFrame::new(w, h, bd, chroma, y, cb, cr)
}

/// A recorded `decode_still` invocation, for asserting the pipeline hands the hook the right
/// inputs.
struct Call {
    payload: Vec<u8>,
    chroma: ChromaFormat,
    bit_depth: u8,
}

#[derive(Default)]
struct Mock {
    calls: Vec<Call>,
}

impl Av1StillDecoder for Mock {
    fn decode_still(&mut self, config: &Av1Config, payload: &[u8]) -> Result<DecodedFrame> {
        // Frame OBU payload = [base, width, height].
        let frame = iter_obus(payload)
            .filter_map(|o| o.ok())
            .find(|o| o.header.obu_type == ObuType::Frame)
            .ok_or(Error::InvalidInput("mock: no frame OBU"))?;
        let (base, w, h) = (
            frame.payload[0],
            u32::from(frame.payload[1]),
            u32::from(frame.payload[2]),
        );
        self.calls.push(Call {
            payload: payload.to_vec(),
            chroma: config.chroma_format(),
            bit_depth: config.bit_depth(),
        });
        build_frame(base, w, h, config.chroma_format(), config.bit_depth())
    }
}

/// A mock that panics if invoked — proves the pipeline rejects a payload before the codec hook.
struct NeverCalled;
impl Av1StillDecoder for NeverCalled {
    fn decode_still(&mut self, _c: &Av1Config, _p: &[u8]) -> Result<DecodedFrame> {
        panic!("decoder must not be invoked");
    }
}

// ---- fixture builders ------------------------------------------------------------------------

/// An `av1C` record for the given chroma layout (`0` mono, `1` 4:2:0, `2` 4:2:2, `3` 4:4:4) and
/// bit depth (8/10/12).
fn av1c_record(chroma_idc: u8, bit_depth: u8) -> Vec<u8> {
    let b1 = if bit_depth == 12 { 2 << 5 } else { 0 }; // 12-bit needs the professional profile
    let depth_bits = match bit_depth {
        10 => 0x40, // high_bitdepth
        12 => 0x60, // high_bitdepth + twelve_bit
        _ => 0x00,  // 8-bit
    };
    let chroma_bits = match chroma_idc {
        0 => 0x1C, // monochrome + subsampling (1, 1)
        1 => 0x0C, // subsampling (1, 1)
        2 => 0x08, // subsampling (1, 0)
        _ => 0x00, // subsampling (0, 0)
    };
    vec![0x81, b1, depth_bits | chroma_bits, 0x00]
}

/// Appends the minimal `leb128()` encoding of `value`.
fn leb128(mut value: usize, out: &mut Vec<u8>) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        if value == 0 {
            out.push(byte);
            break;
        }
        out.push(byte | 0x80);
    }
}

/// A size-fielded OBU of the given type.
fn obu(ty: u8, payload: &[u8]) -> Vec<u8> {
    let mut v = vec![(ty << 3) | 0x02];
    leb128(payload.len(), &mut v);
    v.extend_from_slice(payload);
    v
}

/// A conforming still payload carrying `(base, w, h)` for the mock: a reduced-still-picture
/// sequence header OBU followed by a frame OBU.
fn coded_payload(base: u8, w: u32, h: u32) -> Vec<u8> {
    let mut p = obu(1, &[0x18]); // seq_profile 0 | still_picture 1 | reduced_still_picture 1
    p.extend_from_slice(&obu(6, &[base, w as u8, h as u8]));
    p
}

fn prop_av1c(data: Vec<u8>) -> Property {
    Property {
        essential: true,
        kind: PropertyKind::CodecConfiguration {
            kind: *b"av1C",
            data,
        },
    }
}

fn dimg(to: &[u32]) -> ItemReference {
    ItemReference {
        reference_type: *b"dimg",
        to_item_ids: to.to_vec(),
    }
}

fn base_item(id: u32, item_type: [u8; 4]) -> Item {
    Item {
        id,
        item_type,
        name: String::new(),
        content_type: None,
        content_encoding: None,
        hidden: false,
        references: vec![],
        properties: vec![],
        payload: vec![],
    }
}

/// A coded `av01` item with the given chroma/bit-depth config and `(base, w, h)` payload.
fn coded_item(
    id: u32,
    chroma_idc: u8,
    bd: u8,
    base: u8,
    w: u32,
    h: u32,
    extra: Vec<Property>,
) -> Item {
    let mut properties = vec![prop_av1c(av1c_record(chroma_idc, bd))];
    properties.extend(extra);
    Item {
        properties,
        payload: coded_payload(base, w, h),
        ..base_item(id, *b"av01")
    }
}

fn file(primary_id: u32, items: Vec<Item>) -> Vec<u8> {
    write(&IsoBmffImage {
        major_brand: *b"avif",
        minor_version: 0,
        compatible_brands: vec![*b"avif", *b"mif1", *b"miaf"],
        primary_item_id: primary_id,
        items,
        groups: vec![],
    })
    .expect("write model")
}

// ---- coded-item planar decode ----------------------------------------------------------------

#[test]
fn coded_planar_passes_exact_config_and_payload_and_gradient() {
    let payload = coded_payload(20, 4, 4);
    let item = coded_item(1, 1, 8, 20, 4, 4, vec![]); // 4:2:0, 8-bit
    let bytes = file(1, vec![item]);
    let container = AvifContainer::parse(&bytes).unwrap();

    let mut mock = Mock::default();
    let frame = container.decode_item_planar(1, &mut mock).unwrap();

    // The hook saw the item's exact payload and the config-derived chroma / bit depth.
    assert_eq!(mock.calls.len(), 1);
    assert_eq!(mock.calls[0].payload, payload);
    assert_eq!(mock.calls[0].chroma, ChromaFormat::Yuv420);
    assert_eq!(mock.calls[0].bit_depth, 8);

    // The returned frame is the deterministic gradient, exact.
    assert_eq!((frame.width(), frame.height()), (4, 4));
    assert_eq!(frame.chroma(), ChromaFormat::Yuv420);
    assert_eq!(frame.chroma_dimensions(), (2, 2));
    assert_eq!(frame.dimensions().width, 4);
    for y in 0..4 {
        for x in 0..4 {
            assert_eq!(frame.y()[(y * 4 + x) as usize], ey(20, x, y, 8));
        }
    }
    for cy in 0..2 {
        for cx in 0..2 {
            assert_eq!(frame.cb()[(cy * 2 + cx) as usize], ecb(20, cx, cy, 8));
            assert_eq!(frame.cr()[(cy * 2 + cx) as usize], ecr(20, cx, cy, 8));
        }
    }
}

#[test]
fn non_conforming_payload_never_reaches_the_decoder() {
    // A payload whose only frame precedes its sequence header must be refused by the pipeline
    // (validate_still_payload) before the codec hook is asked to decode it.
    let mut payload = obu(6, &[0x10, 4, 4]);
    payload.extend_from_slice(&obu(1, &[0x18]));
    let item = Item {
        properties: vec![prop_av1c(av1c_record(1, 8))],
        payload,
        ..base_item(1, *b"av01")
    };
    let bytes = file(1, vec![item]);
    let container = AvifContainer::parse(&bytes).unwrap();
    // `NeverCalled` panics if invoked; the error proves it was not.
    assert!(matches!(
        container.decode_item_planar(1, &mut NeverCalled),
        Err(Error::InvalidInput(_))
    ));
}

#[test]
fn essential_unknown_property_is_refused() {
    let bad = Property {
        essential: true,
        kind: PropertyKind::Other {
            kind: *b"a1op", // the layered-image operating-point property (not yet typed)
            data: vec![1],
        },
    };
    let item = coded_item(1, 1, 8, 0, 4, 4, vec![bad]);
    let bytes = file(1, vec![item]);
    let container = AvifContainer::parse(&bytes).unwrap();
    assert!(matches!(
        container.decode_item_planar(1, &mut Mock::default()),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn non_av1_coded_item_is_unsupported() {
    let item = Item {
        properties: vec![Property {
            essential: true,
            kind: PropertyKind::CodecConfiguration {
                kind: *b"hvcC",
                data: vec![1, 2, 3],
            },
        }],
        payload: vec![0xAA],
        ..base_item(1, *b"hvc1")
    };
    let bytes = file(1, vec![item]);
    let container = AvifContainer::parse(&bytes).unwrap();
    assert!(matches!(
        container.decode_item_planar(1, &mut Mock::default()),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn av1_item_without_av1c_is_invalid() {
    let item = Item {
        payload: coded_payload(1, 2, 2),
        ..base_item(1, *b"av01")
    };
    let bytes = file(1, vec![item]);
    let container = AvifContainer::parse(&bytes).unwrap();
    assert!(matches!(
        container.decode_item_planar(1, &mut NeverCalled),
        Err(Error::InvalidInput(_))
    ));
}

// ---- DecodedFrame::new validation ------------------------------------------------------------

#[test]
fn decoded_frame_new_validates_plane_lengths_and_bit_depth() {
    // 4:4:4 — chroma == luma.
    assert!(
        DecodedFrame::new(
            2,
            2,
            8,
            ChromaFormat::Yuv444,
            vec![0; 4],
            vec![0; 4],
            vec![0; 4]
        )
        .is_ok()
    );
    // 4:2:2 — chroma ceil(w/2) x h = 1 x 2.
    assert!(
        DecodedFrame::new(
            2,
            2,
            8,
            ChromaFormat::Yuv422,
            vec![0; 4],
            vec![0; 2],
            vec![0; 2]
        )
        .is_ok()
    );
    // 4:2:0 even — 2 x 2 chroma.
    assert!(
        DecodedFrame::new(
            4,
            4,
            8,
            ChromaFormat::Yuv420,
            vec![0; 16],
            vec![0; 4],
            vec![0; 4]
        )
        .is_ok()
    );
    // 4:2:0 ODD dims — ceiling division: 5x3 luma ⇒ 3x2 chroma = 6.
    assert!(
        DecodedFrame::new(
            5,
            3,
            8,
            ChromaFormat::Yuv420,
            vec![0; 15],
            vec![0; 6],
            vec![0; 6]
        )
        .is_ok()
    );
    assert!(
        DecodedFrame::new(
            5,
            3,
            8,
            ChromaFormat::Yuv420,
            vec![0; 15],
            vec![0; 4],
            vec![0; 4]
        )
        .is_err()
    );
    // Monochrome — chroma must be empty.
    assert!(
        DecodedFrame::new(
            2,
            2,
            8,
            ChromaFormat::Monochrome,
            vec![0; 4],
            vec![],
            vec![]
        )
        .is_ok()
    );
    assert!(
        DecodedFrame::new(
            2,
            2,
            8,
            ChromaFormat::Monochrome,
            vec![0; 4],
            vec![0],
            vec![]
        )
        .is_err()
    );
    // Wrong luma length.
    assert!(
        DecodedFrame::new(
            2,
            2,
            8,
            ChromaFormat::Yuv444,
            vec![0; 3],
            vec![0; 4],
            vec![0; 4]
        )
        .is_err()
    );
    // Bit-depth bounds: 8 and 16 ok; 7 and 17 rejected.
    assert!(
        DecodedFrame::new(
            1,
            1,
            16,
            ChromaFormat::Monochrome,
            vec![0; 1],
            vec![],
            vec![]
        )
        .is_ok()
    );
    assert!(
        DecodedFrame::new(
            1,
            1,
            7,
            ChromaFormat::Monochrome,
            vec![0; 1],
            vec![],
            vec![]
        )
        .is_err()
    );
    assert!(
        DecodedFrame::new(
            1,
            1,
            17,
            ChromaFormat::Monochrome,
            vec![0; 1],
            vec![],
            vec![]
        )
        .is_err()
    );
    // Zero dimensions rejected.
    assert!(DecodedFrame::new(0, 2, 8, ChromaFormat::Monochrome, vec![], vec![], vec![]).is_err());
}

// ---- grid ------------------------------------------------------------------------------------

fn grid_item(id: u32, rows: u16, columns: u16, ow: u32, oh: u32, tiles: &[u32]) -> Item {
    let payload = ImageGrid {
        rows,
        columns,
        output_width: ow,
        output_height: oh,
    }
    .to_bytes()
    .unwrap();
    Item {
        references: vec![dimg(tiles)],
        payload,
        ..base_item(id, *b"grid")
    }
}

/// A hidden monochrome tile of the given base at 2x2.
fn mono_tile(id: u32, base: u8) -> Item {
    Item {
        hidden: true,
        ..coded_item(id, 0, 8, base, 2, 2, vec![])
    }
}

#[test]
fn grid_assembles_row_major_tiles_and_crops() {
    // 2x2 grid of 2x2 monochrome tiles ⇒ 4x4 canvas, cropped to 3x3 output. Distinct bases give
    // each tile a distinct gradient so a swapped/misplaced tile is caught.
    let tiles = [
        mono_tile(2, 10),
        mono_tile(3, 40),
        mono_tile(4, 70),
        mono_tile(5, 100),
    ];
    let bases = [10u8, 40, 70, 100];
    let grid = grid_item(1, 2, 2, 3, 3, &[2, 3, 4, 5]);
    let bytes = file(
        1,
        vec![
            grid,
            tiles[0].clone(),
            tiles[1].clone(),
            tiles[2].clone(),
            tiles[3].clone(),
        ],
    );
    let container = AvifContainer::parse(&bytes).unwrap();

    let frame = container
        .decode_item_planar(1, &mut Mock::default())
        .unwrap();
    assert_eq!((frame.width(), frame.height()), (3, 3));
    assert_eq!(frame.chroma(), ChromaFormat::Monochrome);
    // Independent reference: each output pixel comes from its covering tile's own gradient.
    for oy in 0..3u32 {
        for ox in 0..3u32 {
            let (trow, iy) = (oy / 2, oy % 2);
            let (tcol, ix) = (ox / 2, ox % 2);
            let base = bases[(trow * 2 + tcol) as usize];
            assert_eq!(
                frame.y()[(oy * 3 + ox) as usize],
                ey(base, ix, iy, 8),
                "px ({ox},{oy})"
            );
        }
    }
}

#[test]
fn grid_non_uniform_tiles_are_unsupported() {
    // Mixed tile size.
    let big = Item {
        hidden: true,
        ..coded_item(3, 0, 8, 40, 2, 3, vec![])
    };
    let g = grid_item(1, 1, 2, 4, 2, &[2, 3]);
    let bytes = file(1, vec![g, mono_tile(2, 10), big]);
    let c = AvifContainer::parse(&bytes).unwrap();
    assert!(matches!(
        c.decode_item_planar(1, &mut Mock::default()),
        Err(Error::Unsupported(_))
    ));

    // Mixed chroma.
    let chroma_tile = Item {
        hidden: true,
        ..coded_item(3, 1, 8, 40, 2, 2, vec![])
    };
    let g = grid_item(1, 1, 2, 4, 2, &[2, 3]);
    let bytes = file(1, vec![g, mono_tile(2, 10), chroma_tile]);
    let c = AvifContainer::parse(&bytes).unwrap();
    assert!(matches!(
        c.decode_item_planar(1, &mut Mock::default()),
        Err(Error::Unsupported(_))
    ));

    // Mixed bit depth.
    let deep = Item {
        hidden: true,
        ..coded_item(3, 0, 10, 40, 2, 2, vec![])
    };
    let g = grid_item(1, 1, 2, 4, 2, &[2, 3]);
    let bytes = file(1, vec![g, mono_tile(2, 10), deep]);
    let c = AvifContainer::parse(&bytes).unwrap();
    assert!(matches!(
        c.decode_item_planar(1, &mut Mock::default()),
        Err(Error::Unsupported(_))
    ));
}

#[test]
fn grid_exact_fit_colour_assembles_all_planes() {
    // A 1x2 grid of 2x2 4:4:4 tiles whose output *exactly* fills the 4x2 tiled canvas. Two facets
    // are pinned: (1) the output-vs-canvas comparisons are `>` (strict) — an exact fit must be
    // accepted, killing the `> -> ==`/`>=` mutants; (2) a *colour* grid takes the non-monochrome
    // assembly branch, so the `chroma == Monochrome` test's `== -> !=` inversion (which would
    // drop the chroma planes and fail `DecodedFrame::new`) is caught by asserting exact Cb/Cr
    // samples.
    let color_tile = |id: u32, base: u8| Item {
        hidden: true,
        ..coded_item(id, 3, 8, base, 2, 2, vec![])
    };
    let grid = grid_item(1, 1, 2, 4, 2, &[2, 3]);
    let bytes = file(1, vec![grid, color_tile(2, 10), color_tile(3, 40)]);
    let container = AvifContainer::parse(&bytes).unwrap();
    let frame = container
        .decode_item_planar(1, &mut Mock::default())
        .unwrap();

    assert_eq!((frame.width(), frame.height()), (4, 2));
    assert_eq!(frame.chroma(), ChromaFormat::Yuv444);
    for oy in 0..2u32 {
        for ox in 0..4u32 {
            let base = if ox < 2 { 10 } else { 40 };
            let (ix, iy) = (ox % 2, oy);
            let i = (oy * 4 + ox) as usize;
            assert_eq!(frame.y()[i], ey(base, ix, iy, 8), "y ({ox},{oy})");
            assert_eq!(frame.cb()[i], ecb(base, ix, iy, 8), "cb ({ox},{oy})");
            assert_eq!(frame.cr()[i], ecr(base, ix, iy, 8), "cr ({ox},{oy})");
        }
    }
}

#[test]
fn grid_output_dimension_guard_rejects_each_violation_individually() {
    // One violated condition at a time for `ow == 0 || oh == 0 || ow > cw || oh > ch` on a 1x2
    // grid of 2x2 tiles (canvas 4x2), always asserting the guard's *own* message. Because `&&`
    // binds tighter than `||`, each `|| -> &&` mutation fuses one adjacent pair, so only a
    // fixture violating exactly that one condition exposes it.
    for (ow, oh) in [(0u32, 2u32), (4, 0), (5, 2), (4, 3)] {
        let grid = grid_item(1, 1, 2, ow, oh, &[2, 3]);
        let bytes = file(1, vec![grid, mono_tile(2, 10), mono_tile(3, 40)]);
        let container = AvifContainer::parse(&bytes).unwrap();
        let err = container
            .decode_item_planar(1, &mut Mock::default())
            .unwrap_err();
        assert!(
            err.to_string().contains("output dimensions exceed"),
            "({ow},{oh}): unexpected error: {err}"
        );
    }
}

// ---- iden ------------------------------------------------------------------------------------

#[test]
fn iden_planar_is_a_pure_passthrough() {
    // iden (id 1) → coded source (id 2): the planar surface returns the source frame untouched.
    let source = Item {
        hidden: true,
        ..coded_item(2, 3, 8, 30, 4, 2, vec![])
    };
    let iden = Item {
        references: vec![dimg(&[2])],
        ..base_item(1, *b"iden")
    };
    let bytes = file(1, vec![iden, source]);
    let container = AvifContainer::parse(&bytes).unwrap();

    let via_iden = container
        .decode_item_planar(1, &mut Mock::default())
        .unwrap();
    let via_source = container
        .decode_item_planar(2, &mut Mock::default())
        .unwrap();
    assert_eq!(via_iden, via_source);
}

#[test]
fn iden_requires_exactly_one_source() {
    let iden = Item {
        references: vec![dimg(&[2, 3])],
        ..base_item(1, *b"iden")
    };
    let s = |id| Item {
        hidden: true,
        ..coded_item(id, 0, 8, 1, 2, 2, vec![])
    };
    let bytes = file(1, vec![iden, s(2), s(3)]);
    let container = AvifContainer::parse(&bytes).unwrap();
    assert!(matches!(
        container.decode_item_planar(1, &mut Mock::default()),
        Err(Error::InvalidInput(_))
    ));
}

// ---- overlay is planar-unsupported -----------------------------------------------------------

#[test]
fn overlay_via_planar_is_unsupported() {
    let a = Item {
        hidden: true,
        ..coded_item(2, 3, 8, 10, 2, 2, vec![])
    };
    let ov = Item {
        references: vec![dimg(&[2])],
        payload: ImageOverlay {
            canvas_fill_value: [0; 4],
            output_width: 4,
            output_height: 4,
            offsets: vec![(0, 0)],
        }
        .to_bytes()
        .unwrap(),
        ..base_item(1, *b"iovl")
    };
    let bytes = file(1, vec![ov, a]);
    let container = AvifContainer::parse(&bytes).unwrap();
    assert!(matches!(
        container.decode_item_planar(1, &mut Mock::default()),
        Err(Error::Unsupported(_))
    ));
}

// ---- metadata items are not decodable --------------------------------------------------------

#[test]
fn metadata_item_is_not_a_decodable_image() {
    let exif = Item {
        hidden: true,
        references: vec![ItemReference {
            reference_type: *b"cdsc",
            to_item_ids: vec![1],
        }],
        payload: b"\x00\x00\x00\x00MM\x00*".to_vec(),
        ..base_item(2, *b"Exif")
    };
    let bytes = file(1, vec![coded_item(1, 0, 8, 1, 2, 2, vec![]), exif]);
    let container = AvifContainer::parse(&bytes).unwrap();
    assert!(matches!(
        container.decode_item_planar(2, &mut NeverCalled),
        Err(Error::InvalidInput(_))
    ));
    // A missing id is invalid, too.
    assert!(matches!(
        container.decode_item_planar(99, &mut NeverCalled),
        Err(Error::InvalidInput(_))
    ));
}

// ---- derivation cycles & depth ---------------------------------------------------------------

#[test]
fn mutual_dimg_cycle_errors_without_overflow() {
    // 1 (iden) → 2 (iden) → 1 …
    let a = Item {
        references: vec![dimg(&[2])],
        ..base_item(1, *b"iden")
    };
    let b = Item {
        hidden: true,
        references: vec![dimg(&[1])],
        ..base_item(2, *b"iden")
    };
    let bytes = file(1, vec![a, b]);
    let container = AvifContainer::parse(&bytes).unwrap();
    assert!(matches!(
        container.decode_item_planar(1, &mut Mock::default()),
        Err(Error::InvalidInput(_))
    ));
}

#[test]
fn derivation_depth_is_bounded() {
    // A chain 1→2→…→8 of idens onto a coded leaf 9 exceeds the depth limit; a shallow chain
    // works.
    let mut items = Vec::new();
    for id in 1..=8u32 {
        items.push(Item {
            hidden: id != 1,
            references: vec![dimg(&[id + 1])],
            ..base_item(id, *b"iden")
        });
    }
    items.push(Item {
        hidden: true,
        ..coded_item(9, 0, 8, 5, 2, 2, vec![])
    });
    let bytes = file(1, items);
    let container = AvifContainer::parse(&bytes).unwrap();
    assert!(matches!(
        container.decode_item_planar(1, &mut Mock::default()),
        Err(Error::Unsupported(_))
    ));

    // A 2-deep iden chain onto the same leaf decodes fine.
    let i1 = Item {
        references: vec![dimg(&[2])],
        ..base_item(1, *b"iden")
    };
    let i2 = Item {
        hidden: true,
        references: vec![dimg(&[3])],
        ..base_item(2, *b"iden")
    };
    let leaf = Item {
        hidden: true,
        ..coded_item(3, 0, 8, 5, 2, 2, vec![])
    };
    let bytes = file(1, vec![i1, i2, leaf]);
    let container = AvifContainer::parse(&bytes).unwrap();
    assert!(
        container
            .decode_item_planar(1, &mut Mock::default())
            .is_ok()
    );
}
