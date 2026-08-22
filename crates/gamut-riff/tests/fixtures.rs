//! The reader pinned against byte arrays transcribed by hand from RFC 9649.
//!
//! Every other reader test builds its input with [`gamut_riff::RiffWriter`], so a *shared*
//! misreading of the spec by the reader and the writer would round-trip cleanly and go unnoticed.
//! These fixtures are literal bytes, laid out field by field from §2.3-§2.4 and the example layouts
//! of §2.7.3, so nothing in the crate participates in producing the input.
//!
//! The codestream payloads are stand-ins: this crate parses the container and never looks inside a
//! bitstream, so their contents are irrelevant to what is under test. (`tests/oracle.rs` uses real
//! libwebp-produced codestreams where that matters.)

use gamut_riff::{RiffReader, WebpChunkId, WebpLayout};

/// Asserts the RIFF file-size field of a hand-written fixture is self-consistent: it "counts
/// everything after the size field" (RFC 9649 §2.4), so it must equal `len - 8`. A transcription
/// slip in the tables below then fails here rather than quietly testing something else.
fn assert_well_formed(file: &[u8]) {
    assert_eq!(&file[0..4], b"RIFF", "magic");
    assert_eq!(&file[8..12], b"WEBP", "form");
    let declared = u32::from_le_bytes([file[4], file[5], file[6], file[7]]) as usize;
    assert_eq!(declared, file.len() - 8, "file size field");
    assert_eq!(declared % 2, 0, "§2.4: the RIFF file size is always even");
}

/// §2.6, and the 21-byte prefix restated in §3.4: a simple lossless file is the 12-byte header plus
/// one `VP8L` chunk.
#[rustfmt::skip]
const SIMPLE_LOSSLESS: [u8; 24] = [
    b'R', b'I', b'F', b'F',   // ChunkHeader('RIFF')
    0x10, 0x00, 0x00, 0x00,   // File Size = 16 = 4 ('WEBP') + 8 (chunk header) + 4 (payload)
    b'W', b'E', b'B', b'P',   // form
    b'V', b'P', b'8', b'L',   // FourCC 'VP8L'
    0x04, 0x00, 0x00, 0x00,   // Chunk Size = 4, uint32 little-endian
    0x2f, 0x00, 0x00, 0x00,   // payload: the 0x2f VP8L signature byte, then filler
];

/// §2.7.3 Figure 15 — a lossy image with alpha: `VP8X`, `ALPH`, `VP8 `.
#[rustfmt::skip]
const LOSSY_WITH_ALPHA: [u8; 52] = [
    b'R', b'I', b'F', b'F',
    0x2c, 0x00, 0x00, 0x00,   // File Size = 44
    b'W', b'E', b'B', b'P',
    b'V', b'P', b'8', b'X',   // 'VP8X', 10-byte payload (§2.7 Figure 7)
    0x0a, 0x00, 0x00, 0x00,
    0x10,                     // flags: Rsv|I|L|E|X|A|R with L (alpha) set = 0x10
    0x00, 0x00, 0x00,         // Reserved: 24 bits, MUST be 0
    0x03, 0x00, 0x00,         // Canvas Width Minus One = 3  -> width 4
    0x03, 0x00, 0x00,         // Canvas Height Minus One = 3 -> height 4
    b'A', b'L', b'P', b'H',   // 'ALPH' (§2.7.1.2)
    0x02, 0x00, 0x00, 0x00,
    0x00, 0x11,               // payload: Rsv|P|F|C header byte, then alpha data
    b'V', b'P', b'8', b' ',   // 'VP8 ' — note the trailing space (0x20)
    0x03, 0x00, 0x00, 0x00,
    0x9d, 0x01, 0x2a,         // payload: the VP8 key-frame start code
    0x00,                     // pad byte: odd payload, MUST be 0 (§2.3)
];

/// §2.7.3 Figure 16 — a lossless image followed by an unknown chunk: `VP8X`, `VP8L`, `XYZW`.
#[rustfmt::skip]
const LOSSLESS_WITH_UNKNOWN: [u8; 50] = [
    b'R', b'I', b'F', b'F',
    0x2a, 0x00, 0x00, 0x00,   // File Size = 42
    b'W', b'E', b'B', b'P',
    b'V', b'P', b'8', b'X',
    0x0a, 0x00, 0x00, 0x00,
    0x00,                     // no feature flags
    0x00, 0x00, 0x00,
    0x01, 0x00, 0x00,         // width  = 2
    0x01, 0x00, 0x00,         // height = 2
    b'V', b'P', b'8', b'L',
    0x01, 0x00, 0x00, 0x00,
    0x2f,                     // payload
    0x00,                     // pad byte
    b'X', b'Y', b'Z', b'W',   // an unknown chunk (§2.7.1.6)
    0x02, 0x00, 0x00, 0x00,
    b'h', b'i',
];

/// §2.7.3 Figure 17 — a lossless image with a colour profile and XMP: `VP8X`, `ICCP`, `VP8L`,
/// `XMP `.
#[rustfmt::skip]
const LOSSLESS_WITH_ICC_AND_XMP: [u8; 64] = [
    b'R', b'I', b'F', b'F',
    0x38, 0x00, 0x00, 0x00,   // File Size = 56
    b'W', b'E', b'B', b'P',
    b'V', b'P', b'8', b'X',
    0x0a, 0x00, 0x00, 0x00,
    0x24,                     // flags: I (ICC, 0x20) | X (XMP, 0x04)
    0x00, 0x00, 0x00,
    0x0f, 0x00, 0x00,         // width  = 16
    0x07, 0x00, 0x00,         // height = 8
    b'I', b'C', b'C', b'P',   // §2.7.1.4 — MUST appear before the image data
    0x03, 0x00, 0x00, 0x00,
    b'i', b'c', b'c',
    0x00,                     // pad byte
    b'V', b'P', b'8', b'L',
    0x02, 0x00, 0x00, 0x00,
    0x2f, 0x00,
    b'X', b'M', b'P', b' ',   // 'XMP ' — trailing space (§2.7.1.5)
    0x04, 0x00, 0x00, 0x00,
    b'<', b'x', b'/', b'>',
];

#[test]
fn every_fixture_is_self_consistent() {
    for file in [
        &SIMPLE_LOSSLESS[..],
        &LOSSY_WITH_ALPHA[..],
        &LOSSLESS_WITH_UNKNOWN[..],
        &LOSSLESS_WITH_ICC_AND_XMP[..],
    ] {
        assert_well_formed(file);
    }
}

#[test]
fn reads_the_simple_lossless_layout() {
    let layout = WebpLayout::parse(&SIMPLE_LOSSLESS).expect("parse");
    assert_eq!(layout.vp8x, None);
    assert_eq!(
        layout.bitstream,
        Some((WebpChunkId::Vp8l, &[0x2f, 0x00, 0x00, 0x00][..]))
    );
    assert!(layout.metadata.is_empty());
    assert!(layout.unknown.is_empty());
}

#[test]
fn reads_figure_15_lossy_with_alpha() {
    let layout = WebpLayout::parse(&LOSSY_WITH_ALPHA).expect("parse");
    let vp8x = layout.vp8x.expect("VP8X present");
    assert!(vp8x.alpha, "the L flag decodes as alpha");
    assert!(!vp8x.icc_profile && !vp8x.exif_metadata && !vp8x.xmp_metadata && !vp8x.animation);
    assert_eq!(
        (vp8x.canvas_width, vp8x.canvas_height),
        (4, 4),
        "the canvas fields are stored 1-based"
    );
    assert_eq!(layout.alph, Some(&[0x00, 0x11][..]));
    assert_eq!(
        layout.bitstream,
        Some((WebpChunkId::Vp8, &[0x9d, 0x01, 0x2a][..])),
        "the pad byte is framing and never reaches the payload"
    );
}

#[test]
fn reads_figure_16_unknown_chunk() {
    let layout = WebpLayout::parse(&LOSSLESS_WITH_UNKNOWN).expect("parse");
    assert_eq!(layout.bitstream, Some((WebpChunkId::Vp8l, &[0x2f][..])));
    assert_eq!(layout.unknown.len(), 1);
    assert_eq!(layout.unknown[0].fourcc.as_bytes(), b"XYZW");
    assert_eq!(layout.unknown[0].payload, b"hi");
}

#[test]
fn reads_figure_17_icc_and_xmp() {
    let layout = WebpLayout::parse(&LOSSLESS_WITH_ICC_AND_XMP).expect("parse");
    let vp8x = layout.vp8x.expect("VP8X present");
    assert!(vp8x.icc_profile && vp8x.xmp_metadata);
    assert!(!vp8x.alpha && !vp8x.exif_metadata && !vp8x.animation);
    assert_eq!((vp8x.canvas_width, vp8x.canvas_height), (16, 8));
    assert_eq!(layout.metadata.icc, Some(&b"icc"[..]));
    assert_eq!(layout.metadata.xmp, Some(&b"<x/>"[..]));
    assert_eq!(layout.metadata.exif, None);
    assert_eq!(
        layout.bitstream,
        Some((WebpChunkId::Vp8l, &[0x2f, 0x00][..]))
    );
}

#[test]
fn the_low_level_reader_sees_the_same_chunk_sequence() {
    // The strict layout view and the permissive iterator must agree on the framing; only their
    // interpretation differs.
    let fourccs: Vec<[u8; 4]> = RiffReader::new(&LOSSLESS_WITH_ICC_AND_XMP)
        .expect("header")
        .map(|c| *c.expect("chunk").fourcc.as_bytes())
        .collect();
    assert_eq!(fourccs, vec![*b"VP8X", *b"ICCP", *b"VP8L", *b"XMP "]);
}

#[test]
fn a_hand_written_odd_payload_is_padded_to_an_even_boundary() {
    // §2.3: "If Chunk Size is odd, a single padding byte -- which MUST be 0 -- is added." Both
    // fixtures with an odd payload carry that byte, and the total file stays even.
    for file in [&LOSSY_WITH_ALPHA[..], &LOSSLESS_WITH_UNKNOWN[..]] {
        assert_eq!(file.len() % 2, 0);
        assert!(WebpLayout::parse(file).is_ok());
    }
    // The `VP8 ` payload of Figure 15 is 3 bytes at offset 44..47, so byte 47 is its pad byte.
    assert_eq!(LOSSY_WITH_ALPHA[47], 0x00);
}
