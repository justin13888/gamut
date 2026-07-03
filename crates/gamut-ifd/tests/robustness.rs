//! Robustness corpus: the `#![forbid(unsafe_code)]` reader is offset-driven — a classic
//! parser-exploit surface — so hostile input must yield a typed error or a valid parse, never a
//! panic, a hang, or unbounded allocation (STATUS P6).

use gamut_ifd::{ByteOrder, Ifd, TiffFile, Value, Variant, read, read_tree, write};

/// Sub-IFD pointer tags a DNG/EXIF-shaped consumer would follow.
const POINTER_TAGS: &[u16] = &[330, 34665, 34853];

/// A representative stream: two chained IFDs, inline and out-of-line values of several types, and
/// a sub-IFD tree two levels deep.
fn valid_stream(variant: Variant) -> Vec<u8> {
    let mut grandchild = Ifd::new();
    grandchild.set(33434, Value::Rational(vec![(1, 200)]));
    let mut child = Ifd::new();
    child.set(256, Value::Short(vec![16]));
    child.set(258, Value::Short(vec![8, 8, 8])); // out of line (classic)
    child.set_sub_ifd(34665, vec![grandchild]);
    let mut first = Ifd::new();
    first.set(256, Value::Short(vec![640]));
    first.set(270, Value::Ascii("first\0second".to_owned())); // multi-string, out of line
    first.set(282, Value::Rational(vec![(300, 1)]));
    first.set_sub_ifd(330, vec![child]);
    let mut second = Ifd::new();
    second.set(256, Value::Long(vec![9]));
    write(&TiffFile {
        order: ByteOrder::LittleEndian,
        variant,
        ifds: vec![first, second],
    })
    .expect("write")
}

/// Both readers must survive `data` without panicking; the parse may succeed or fail.
fn survives(data: &[u8]) {
    let _ = read(data);
    let _ = read_tree(data, POINTER_TAGS);
}

#[test]
fn specific_malformed_inputs_error_without_panic() {
    let cases: &[&[u8]] = &[
        b"",
        b"II",
        b"II\x2a\x00",
        b"XX\x2a\x00\x08\x00\x00\x00",     // bad byte-order mark
        b"II\x00\x00\x08\x00\x00\x00",     // bad magic
        b"MM\x00\x2a\xff\xff\xff\x7f",     // first-IFD offset past EOF (big-endian)
        b"II\x2a\x00\x08\x00\x00\x00",     // first IFD at EOF
        b"II\x2a\x00\x08\x00\x00\x00\xff", // truncated IFD count
        b"II\x2a\x00\x00\x00\x00\x00",     // first-IFD offset 0 (no IFD)
        // A 1-entry IFD whose value count is huge (byte-length overflow path), then truncated.
        b"II\x2a\x00\x08\x00\x00\x00\x01\x00\x00\x01\x03\x00\xff\xff\xff\xff\x08\x00\x00\x00\x00\x00\x00\x00",
        // An IFD whose next-IFD pointer loops back to itself.
        b"II\x2a\x00\x08\x00\x00\x00\x00\x00\x08\x00\x00\x00",
    ];
    for &c in cases {
        survives(c);
    }
    // The loop case must be a typed error, not a hang.
    assert!(read(b"II\x2a\x00\x08\x00\x00\x00\x00\x00\x08\x00\x00\x00").is_err());
}

#[test]
fn truncations_do_not_panic() {
    let valid = valid_stream(Variant::Classic);
    assert!(read_tree(&valid, POINTER_TAGS).is_ok());
    for len in 0..valid.len() {
        survives(&valid[..len]);
    }
}

#[test]
fn byte_flip_fuzz_does_not_panic() {
    let valid = valid_stream(Variant::Classic);
    // Deterministic LCG (no RNG dependency) drives the mutations.
    let mut state: u64 = 0x1234_5678_9abc_def0;
    let mut next = || {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (state >> 33) as u32
    };
    for _ in 0..5000 {
        let mut data = valid.clone();
        let flips = 1 + next() % 4;
        for _ in 0..flips {
            let pos = next() as usize % data.len();
            data[pos] ^= (next() & 0xff) as u8;
        }
        survives(&data);
    }
}

/// Every single-byte overwrite of every position with a boundary value — cheap exhaustive
/// coverage of the offset words, counts, and type codes that drive the parse.
#[test]
fn single_byte_overwrites_do_not_panic() {
    let valid = valid_stream(Variant::Classic);
    for pos in 0..valid.len() {
        for byte in [0x00, 0x01, 0x7f, 0x80, 0xff] {
            let mut data = valid.clone();
            data[pos] = byte;
            survives(&data);
        }
    }
}

#[cfg(feature = "bigtiff")]
mod bigtiff {
    use super::*;

    #[test]
    fn bigtiff_truncations_do_not_panic() {
        let valid = valid_stream(Variant::Big);
        assert!(read_tree(&valid, POINTER_TAGS).is_ok());
        for len in 0..valid.len() {
            survives(&valid[..len]);
        }
    }

    #[test]
    fn bigtiff_single_byte_overwrites_do_not_panic() {
        let valid = valid_stream(Variant::Big);
        for pos in 0..valid.len() {
            for byte in [0x00, 0x01, 0x7f, 0x80, 0xff] {
                let mut data = valid.clone();
                data[pos] = byte;
                survives(&data);
            }
        }
    }
}
