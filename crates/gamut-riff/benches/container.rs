//! RIFF (WebP) container throughput benchmarks (issue #149).
//!
//! Intentionally tight: writing a multi-chunk WebP RIFF stream (header + per-chunk framing +
//! file-size back-patch) and reading it back by iterating every chunk. Counters report payload
//! bytes per second. Run with `cargo bench -p gamut-riff`.

use divan::counter::BytesCount;
use divan::{Bencher, black_box};
use gamut_riff::{FourCc, RiffReader, RiffWriter};

fn main() {
    divan::main();
}

/// Number of chunks per file and the payload size of each.
const CHUNKS: usize = 64;
const PAYLOAD: usize = 4096;

fn fourccs() -> [FourCc; 4] {
    [FourCc::VP8L, FourCc::EXIF, FourCc::XMP, FourCc::ICCP]
}

fn build_file() -> Vec<u8> {
    let payload = vec![0x5Au8; PAYLOAD];
    let codes = fourccs();
    let mut w = RiffWriter::new();
    for i in 0..CHUNKS {
        w.write_chunk(codes[i % codes.len()], &payload);
    }
    w.finish()
}

#[divan::bench]
fn write_chunks(bencher: Bencher) {
    let payload = vec![0x5Au8; PAYLOAD];
    let codes = fourccs();
    bencher
        .counter(BytesCount::new(CHUNKS * PAYLOAD))
        .bench_local(|| {
            let mut w = RiffWriter::new();
            for i in 0..CHUNKS {
                w.write_chunk(codes[i % codes.len()], black_box(&payload));
            }
            w.finish()
        });
}

#[divan::bench]
fn read_chunks(bencher: Bencher) {
    let file = build_file();
    bencher
        .counter(BytesCount::new(file.len()))
        .bench_local(|| {
            let mut bytes = 0usize;
            for chunk in RiffReader::new(black_box(&file)).unwrap() {
                bytes += chunk.unwrap().payload.len();
            }
            bytes
        });
}
