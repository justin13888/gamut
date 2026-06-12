//! Robustness: the `#![forbid(unsafe_code)]` decoder must reject malformed input cleanly — never
//! panic, never allocate unboundedly — on hostile data (P19).

use gamut_core::Dimensions;
use gamut_tiff::{Compression, TiffDecoder, TiffEncoder};

fn valid_lzw_tiff() -> Vec<u8> {
    let dims = Dimensions {
        width: 12,
        height: 9,
    };
    let rgb: Vec<u8> = (0..12 * 9 * 3).map(|i| (i * 7) as u8).collect();
    let mut out = Vec::new();
    TiffEncoder::new()
        .with_compression(Compression::Lzw)
        .encode_rgb8(&rgb, dims, &mut out)
        .expect("encode");
    out
}

#[test]
fn specific_malformed_inputs_error_without_panic() {
    let dec = TiffDecoder::new();
    let mut out = Vec::new();
    let cases: &[&[u8]] = &[
        b"",
        b"II",
        b"II\x2a\x00",
        b"XX\x2a\x00\x08\x00\x00\x00",     // bad byte-order mark
        b"II\x00\x00\x08\x00\x00\x00",     // bad magic
        b"II\x2a\x00\xff\xff\xff\x7f",     // first-IFD offset past EOF
        b"II\x2a\x00\x08\x00\x00\x00\xff", // truncated IFD
        // A 1-entry IFD whose ImageWidth claims a huge value, then truncated.
        b"II\x2a\x00\x08\x00\x00\x00\x01\x00\x00\x01\x04\x00\x01\x00\x00\x00\xff\xff\xff\x7f\x00\x00\x00\x00",
    ];
    for &c in cases {
        out.clear();
        // Must return without panicking (Ok or Err either way).
        let _ = dec.decode_to_rgb8(c, &mut out);
    }
}

#[test]
fn truncations_do_not_panic() {
    let valid = valid_lzw_tiff();
    let dec = TiffDecoder::new();
    let mut out = Vec::new();
    for len in 0..=valid.len() {
        out.clear();
        let _ = dec.decode_to_rgb8(&valid[..len], &mut out);
    }
}

#[test]
fn byte_flip_fuzz_does_not_panic() {
    let valid = valid_lzw_tiff();
    let dec = TiffDecoder::new();
    let mut out = Vec::new();
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
        out.clear();
        let _ = dec.decode_to_rgb8(&data, &mut out);
    }
}
