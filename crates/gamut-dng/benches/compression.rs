//! DNG Deflate (`Compression = 8`) ratio + throughput benchmarks (issue #196).
//!
//! DNG's ZIP codec compresses the *packed sample stream*, so the inputs here are packed raw
//! payloads rather than pixels — that is what the encoder actually sees, and raw sensor data is
//! far less compressible than the photographic or textual data a general DEFLATE benchmark uses.
//!
//! `cargo bench -p gamut-dng` first prints a size table comparing the shipped encoder
//! (`gamut-deflate` at `Level::Default`) against the `miniz_oxide` level 6 it replaced and against
//! `gamut-deflate`'s space-optimal `Level::Best`, then runs divan throughput benchmarks over the
//! same three — so the ratio win and its speed cost are both visible.

use divan::counter::BytesCount;
use divan::{Bencher, black_box};
use gamut_deflate::{DeflateEncoder, Level};
use gamut_dng::DngDecoder;

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

/// Packs 16-bit samples little-endian, the layout `bitpack::pack` writes at that depth.
fn pack16(samples: &[u16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for &s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// A sensor-like mosaic: a smooth illumination falloff, a per-CFA-channel gain, and deterministic
/// per-photosite noise. The noise is what makes raw hard to compress, so a clean synthetic gradient
/// would flatter every codec equally and measure nothing useful.
fn sensor_cfa(width: usize, height: usize, bits: u16) -> Vec<u16> {
    let max = ((1u32 << bits) - 1) as f64;
    let mut samples = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            // Cosine-fourth-law-ish falloff from the frame centre.
            let dx = (x as f64 / width as f64) - 0.5;
            let dy = (y as f64 / height as f64) - 0.5;
            let falloff = 1.0 - 1.4 * (dx * dx + dy * dy);
            // RGGB: green photosites collect roughly twice what red and blue do.
            let gain = match (x % 2, y % 2) {
                (0, 0) => 0.42, // R
                (1, 1) => 0.31, // B
                _ => 0.70,      // G
            };
            // Deterministic shot-noise stand-in, a few percent of full scale.
            let hash = ((y * width + x) as u32).wrapping_mul(2_654_435_761) >> 11;
            let noise = (hash % 2048) as f64 / 2048.0 - 0.5;
            let value = (falloff * gain + noise * 0.05).clamp(0.0, 1.0) * max;
            samples.push(value as u16);
        }
    }
    samples
}

/// How much of a real sample's packed payload to measure. Adobe's samples run to hundreds of
/// megabytes decoded; a leading slice is representative of the codec's behaviour and keeps
/// `cargo bench --workspace` affordable.
const SAMPLE_SLICE: usize = 16 << 20;

/// How many of Adobe's sample DNGs to include.
const SAMPLE_COUNT: usize = 3;

/// Adobe's own sample DNGs, decoded and repacked at 16-bit — real sensor data, which is the only
/// input that answers "what does this cost a real DNG?". Empty if the SDK sample corpus the oracle
/// extracts is unavailable.
fn adobe_samples() -> Vec<(String, Vec<u8>)> {
    let Ok(entries) = std::fs::read_dir(gamut_dng_oracle::sample_files_dir()) else {
        return Vec::new();
    };
    let mut paths: Vec<_> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "dng"))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for path in paths {
        if out.len() == SAMPLE_COUNT {
            break;
        }
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let Ok(decoded) = DngDecoder::new().decode(&bytes) else {
            continue;
        };
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let mut packed = pack16(decoded.raw.samples());
        packed.truncate(SAMPLE_SLICE);
        out.push((format!("{name} (first {} MiB)", packed.len() >> 20), packed));
    }
    out
}

/// Packed raw payloads spanning the shapes the DNG Deflate path encodes.
fn corpus() -> Vec<(String, Vec<u8>)> {
    let mut inputs = vec![
        ("cfa16 1024x768".into(), pack16(&sensor_cfa(1024, 768, 16))),
        (
            "cfa8 1024x768".into(),
            sensor_cfa(1024, 768, 8).iter().map(|&s| s as u8).collect(),
        ),
        (
            // LinearRaw interleaves its three planes, so it packs as one wider row of samples.
            "linear16 512x384".into(),
            pack16(&sensor_cfa(512 * 3, 384, 16)),
        ),
    ];
    inputs.extend(adobe_samples());
    inputs
}

/// A 256x256 tile of 16-bit samples — the chunk size a tiling DNG writer typically hands the codec.
const TILE_BYTES: usize = 256 * 256 * 2;

/// Total zlib bytes when `data` is compressed as independent chunks of `chunk` bytes, as DNG does
/// per strip or tile. `None` compresses the payload whole, which is what gamut's untiled encoder
/// does — it writes one strip for the entire image (`RowsPerStrip = ImageLength`).
fn compressed_len(compress: &dyn Fn(&[u8]) -> Vec<u8>, data: &[u8], chunk: Option<usize>) -> usize {
    match chunk {
        None => compress(data).len(),
        Some(chunk) => data.chunks(chunk).map(|part| compress(part).len()).sum(),
    }
}

/// Prints a size comparison (zlib streams; lower is better) against the `miniz_oxide` level 6 that
/// `gamut-deflate` replaced.
///
/// Chunking is reported both ways because it used to decide whether `Level::Best`'s optimal parse
/// engaged at all: before #343 that parse ran only on inputs of 1 MiB or less, so a whole-image
/// strip above it fell back to lazy matching while a tiled writer's chunks stayed under it. The
/// parse now spans large input instead, so both rows exercise it.
fn print_size_table() {
    println!(
        "\nDNG Deflate: zlib-stream output size, bytes (lower is better):\n\n{:<44} {:>11} {:>11} {:>11} {:>11}  {:>10}",
        "packed payload / chunking", "raw", "miniz-6", "Default", "Best", "Default/m6"
    );
    for (name, data) in corpus() {
        for (label, chunk) in [("1 strip", None), ("256x256 tiles", Some(TILE_BYTES))] {
            let miniz = compressed_len(
                &|part| miniz_oxide::deflate::compress_to_vec_zlib(part, 6),
                &data,
                chunk,
            );
            let default = compressed_len(&|part| gamut_zlib(Level::Default, part), &data, chunk);
            let best = compressed_len(&|part| gamut_zlib(Level::Best, part), &data, chunk);
            let delta = (default as f64 / miniz.max(1) as f64 - 1.0) * 100.0;
            println!(
                "{:<44} {:>11} {:>11} {:>11} {:>11}  {:>9.1}%",
                format!("{name} / {label}"),
                data.len(),
                miniz,
                default,
                best,
                delta,
            );
        }
    }
    println!();
}

/// A single representative payload for the throughput benchmarks: one 16-bit CFA frame, sized so
/// the `Level::Best` optimal parse stays quick (one span, under the default 1 MiB limit).
fn throughput_input() -> Vec<u8> {
    pack16(&sensor_cfa(512, 384, 16))
}

/// Encode throughput for the shipped level and the space-optimal one: input bytes per second.
#[divan::bench(args = [Level::Default, Level::Best])]
fn gamut_deflate(bencher: Bencher, level: Level) {
    let data = throughput_input();
    bencher
        .counter(BytesCount::new(data.len()))
        .bench_local(|| gamut_zlib(black_box(level), black_box(&data)));
}

/// Baseline: the `miniz_oxide` level 6 this crate encoded with before issue #196.
#[divan::bench]
fn miniz_oxide_6(bencher: Bencher) {
    let data = throughput_input();
    bencher
        .counter(BytesCount::new(data.len()))
        .bench_local(|| miniz_oxide::deflate::compress_to_vec_zlib(black_box(&data), 6));
}
