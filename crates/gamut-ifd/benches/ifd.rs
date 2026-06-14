//! IFD container read/write throughput benchmarks (issue #149).
//!
//! Intentionally tight: the two-pass offset-laying writer and the offset-driven reader over a
//! realistic single-IFD TIFF directory (baseline tags plus sizeable out-of-line `StripOffsets`/
//! `StripByteCounts` arrays). Counters report serialized bytes per second. Run with
//! `cargo bench -p gamut-ifd`.

use divan::counter::BytesCount;
use divan::{Bencher, black_box};
use gamut_ifd::{ByteOrder, Ifd, TiffFile, Value, Variant, read, write};

fn main() {
    divan::main();
}

/// Number of strips, which sizes the out-of-line `StripOffsets`/`StripByteCounts` pools.
const STRIPS: usize = 64;

/// A representative baseline-TIFF directory: small inline scalars plus two long out-of-line
/// arrays, so the writer's layout/back-patch passes and the reader's offset chase both do work.
fn sample_file() -> TiffFile {
    let mut ifd = Ifd::new();
    ifd.set(256, Value::Short(vec![4096])); // ImageWidth
    ifd.set(257, Value::Short(vec![4096])); // ImageLength
    ifd.set(258, Value::Short(vec![8, 8, 8])); // BitsPerSample
    ifd.set(259, Value::Short(vec![1])); // Compression (none)
    ifd.set(262, Value::Short(vec![2])); // PhotometricInterpretation (RGB)
    ifd.set(
        273,
        Value::Long((0..STRIPS as u32).map(|i| 1024 + i * 4096).collect()),
    ); // StripOffsets
    ifd.set(277, Value::Short(vec![3])); // SamplesPerPixel
    ifd.set(278, Value::Short(vec![64])); // RowsPerStrip
    ifd.set(279, Value::Long(vec![4096; STRIPS])); // StripByteCounts
    ifd.set(282, Value::Rational(vec![(300, 1)])); // XResolution
    ifd.set(283, Value::Rational(vec![(300, 1)])); // YResolution
    ifd.set(296, Value::Short(vec![2])); // ResolutionUnit (inch)
    ifd.set(305, Value::Ascii("gamut-tiff benchmark".to_owned())); // Software
    ifd.set(306, Value::Ascii("2026:06:13 00:00:00".to_owned())); // DateTime
    TiffFile {
        order: ByteOrder::LittleEndian,
        variant: Variant::Classic,
        ifds: vec![ifd],
    }
}

#[divan::bench]
fn write_ifd(bencher: Bencher) {
    let file = sample_file();
    bencher
        .counter(BytesCount::new(write(&file).len()))
        .bench_local(|| write(black_box(&file)));
}

#[divan::bench]
fn read_ifd(bencher: Bencher) {
    let bytes = write(&sample_file());
    bencher
        .counter(BytesCount::new(bytes.len()))
        .bench_local(|| read(black_box(&bytes)).unwrap());
}
