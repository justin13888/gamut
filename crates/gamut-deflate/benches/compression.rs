//! Compression ratio + throughput benchmarks for `gamut-deflate` (issues #149, #195).
//!
//! For a space-optimizing encoder two things matter: the ratio it achieves and the time it costs.
//! `cargo bench -p gamut-deflate` first prints a size table comparing every level against `zlib -9`,
//! `miniz_oxide` (max), and the `zopfli` crate, then runs divan throughput benchmarks per level plus
//! the two external baselines — so the ratio win and its speed cost are both visible.

use divan::counter::BytesCount;
use divan::{Bencher, black_box};
use gamut_deflate::{DeflateEncoder, Level};

fn main() {
    print_size_table();
    divan::main();
}

fn gamut_zlib(level: Level, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    DeflateEncoder::new()
        .with_level(level)
        .zlib_compress(data, &mut out);
    out
}

fn zopfli_zlib(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    zopfli::compress(
        zopfli::Options::default(),
        zopfli::Format::Zlib,
        data,
        &mut out,
    )
    .expect("zopfli writes to an in-memory Vec, which cannot fail");
    out
}

/// A representative corpus: real spec text and a source file (both compressible real-world data),
/// plus synthetic edge cases spanning byte statistics.
fn corpus() -> Vec<(String, Vec<u8>)> {
    let mut inputs: Vec<(String, Vec<u8>)> = vec![
        (
            "text.x300".into(),
            b"the quick brown fox jumps over the lazy dog. ".repeat(300),
        ),
        (
            "ramp20k".into(),
            (0..20_000u32).map(|i| (i % 256) as u8).collect(),
        ),
        (
            "pseudo20k".into(),
            (0..20_000u32)
                .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
                .collect(),
        ),
    ];
    for rel in ["../../references/png/rfc1951.txt", "src/lz77.rs"] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(rel);
        if let Ok(bytes) = std::fs::read(&path) {
            let name = rel.rsplit('/').next().unwrap_or(rel).to_string();
            inputs.push((name, bytes));
        }
    }
    inputs
}

/// Prints a size comparison (zlib streams; lower is better) reproducing the `README.md` table.
fn print_size_table() {
    println!(
        "\nzlib-stream output size, bytes (lower is better):\n\n{:<14} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}  {:>10}",
        "input", "raw", "Default", "Best", "zlib-9", "miniz-10", "zopfli", "Best/z9"
    );
    for (name, data) in corpus() {
        let best = gamut_zlib(Level::Best, &data);
        let z9 = zlib_oracle::compress(&data, 9).expect("zlib-9 compresses");
        let ratio = (best.len() as f64 / z9.len().max(1) as f64 - 1.0) * 100.0;
        println!(
            "{:<14} {:>9} {:>9} {:>9} {:>9} {:>9} {:>9}  {:>9.1}%",
            name,
            data.len(),
            gamut_zlib(Level::Default, &data).len(),
            best.len(),
            z9.len(),
            miniz_oxide::deflate::compress_to_vec_zlib(&data, 10).len(),
            zopfli_zlib(&data).len(),
            ratio,
        );
    }
    println!();
}

/// A single medium, real-world input for the throughput benchmarks (kept modest so the
/// 15-iteration `zopfli` baseline stays quick). Falls back to synthetic text if the file is absent.
fn throughput_input() -> Vec<u8> {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../references/png/rfc1950.txt");
    std::fs::read(path)
        .unwrap_or_else(|_| b"the quick brown fox jumps over the lazy dog. ".repeat(400))
}

/// Encode throughput per level: bytes of input compressed per second.
#[divan::bench(args = [Level::Store, Level::Fast, Level::Default, Level::Best])]
fn gamut(bencher: Bencher, level: Level) {
    let data = throughput_input();
    bencher
        .counter(BytesCount::new(data.len()))
        .bench_local(|| gamut_zlib(black_box(level), black_box(&data)));
}

/// Baseline: `miniz_oxide` at its maximum level (the fast/general pure-Rust engine).
#[divan::bench]
fn miniz_oxide_max(bencher: Bencher) {
    let data = throughput_input();
    bencher
        .counter(BytesCount::new(data.len()))
        .bench_local(|| miniz_oxide::deflate::compress_to_vec_zlib(black_box(&data), 10));
}

/// Baseline: the `zopfli` crate at its defaults (the state-of-the-art space-optimal encoder).
#[divan::bench]
fn zopfli(bencher: Bencher) {
    let data = throughput_input();
    bencher
        .counter(BytesCount::new(data.len()))
        .bench_local(|| zopfli_zlib(black_box(&data)));
}
