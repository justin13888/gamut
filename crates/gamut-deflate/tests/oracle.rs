//! Differential cross-checks for gamut-deflate against the reference C **zlib**.
//!
//! gamut ships no inflater, so correctness is proven by inflating the encoder's output with zlib and
//! asserting it reproduces the original bytes. Every lossless path must round-trip exactly, for both
//! raw DEFLATE and the zlib wrapper, across edge cases and varied byte statistics.

use gamut_deflate::{DeflateEncoder, Level};

const LEVELS: &[Level] = &[Level::Store, Level::Fast, Level::Default, Level::Best];

/// Deterministic inputs covering edge cases and a spread of byte statistics.
fn corpus() -> Vec<Vec<u8>> {
    vec![
        Vec::new(),                              // empty
        vec![0x00],                              // single byte
        vec![0xAB; 1000],                        // long run
        (0..4096u32).map(|i| i as u8).collect(), // repeating ramp
        (0..5000u32)
            .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
            .collect(), // pseudo-random
        b"the quick brown fox jumps over the lazy dog. ".repeat(50), // english text
        vec![0u8; 70_000],                       // > 64 KiB: multi-block
    ]
}

#[test]
fn raw_deflate_round_trips_via_zlib() {
    for data in corpus() {
        for &level in LEVELS {
            let mut out = Vec::new();
            DeflateEncoder::new()
                .with_level(level)
                .compress(&data, &mut out);
            let back = zlib_oracle::inflate_raw(&out).unwrap_or_else(|e| {
                panic!(
                    "inflate_raw failed (level {level:?}, {} bytes): {e}",
                    data.len()
                )
            });
            assert_eq!(
                back,
                data,
                "raw round-trip mismatch at level {level:?}, {} bytes",
                data.len()
            );
        }
    }
}

#[test]
fn back_references_compress_and_round_trip() {
    // Highly repetitive input must shrink dramatically once LZ77 matches are emitted — and still
    // inflate back to the original through the reference decoder.
    let data = b"the quick brown fox jumps over the lazy dog. ".repeat(200);
    for &level in &[Level::Fast, Level::Default, Level::Best] {
        let mut out = Vec::new();
        DeflateEncoder::new()
            .with_level(level)
            .zlib_compress(&data, &mut out);
        assert_eq!(zlib_oracle::inflate_zlib(&out).unwrap(), data);
        assert!(
            out.len() < data.len() / 4,
            "level {level:?}: {} should be far smaller than {}",
            out.len(),
            data.len()
        );
    }
}

#[test]
fn zlib_stream_round_trips_via_zlib() {
    for data in corpus() {
        for &level in LEVELS {
            let mut out = Vec::new();
            DeflateEncoder::new()
                .with_level(level)
                .zlib_compress(&data, &mut out);
            let back = zlib_oracle::inflate_zlib(&out)
                .unwrap_or_else(|e| panic!("inflate_zlib failed (level {level:?}): {e}"));
            assert_eq!(
                back,
                data,
                "zlib round-trip mismatch at level {level:?}, {} bytes",
                data.len()
            );
        }
    }
}
