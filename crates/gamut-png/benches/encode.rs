//! PNG encode size and throughput benchmarks (issues #224, #149).
//!
//! For a space-efficient encoder two things matter, and they trade against each other: the size it
//! achieves and the time it costs. So `cargo bench -p gamut-png` first prints two tables -- output
//! size and bits-per-pixel against libpng at maximum compression, then where the bytes went stage
//! by stage -- and only then runs the divan throughput benchmarks.
//!
//! Both tables are computed through [`gamut_png::deconstruct`], which reads any PNG whoever wrote
//! it. That is what makes the libpng column a like-for-like comparison rather than two encoders'
//! self-reports, and it is why the stage table can attribute a size difference to filtering, to
//! the colour-type choice, or to DEFLATE.
//!
//! Counters report bytes of *source* pixels per second, so figures are comparable with the other
//! codec suites. Run with `cargo bench -p gamut-png` (or `mise run bench`); add
//! `--features test-support` for the per-stage rows.

use divan::counter::BytesCount;
use divan::{Bencher, black_box};
use gamut_core::{Dimensions, EncodeImage, ImageRef, Rgb8, Rgba8};
use gamut_png::{FilterStrategy, FilterType, Level, PngEncoder, deconstruct};

fn main() {
    print_size_table();
    print_stage_table();
    divan::main();
}

/// Side length of the square test images.
///
/// 256 is the floor that means anything here: RGB at 256x256 is 192 KiB, roughly six times the
/// 32 KiB DEFLATE window, so LZ77 match behaviour is real. A 64x64 image fits *inside* the window
/// and would flatter both encoders equally, hiding the thing being measured.
const SIDE: u32 = 256;

/// What a corpus entry is: named pixels in one of the two layouts the tables exercise.
enum Pixels {
    /// 8-bit RGB, `SIDE x SIDE`.
    Rgb(Vec<u8>),
    /// 8-bit RGBA, `SIDE x SIDE`.
    Rgba(Vec<u8>),
}

/// One named corpus entry.
struct Case {
    /// Short name, used as the table's row label and the divan argument.
    name: &'static str,
    /// Image width in pixels.
    width: u32,
    /// Image height in pixels.
    height: u32,
    /// The samples.
    pixels: Pixels,
}

impl Case {
    /// Raw sample bytes -- the denominator every ratio in the tables is read against.
    fn raw_len(&self) -> usize {
        match &self.pixels {
            Pixels::Rgb(v) | Pixels::Rgba(v) => v.len(),
        }
    }

    /// libpng's colour-type code for this entry's layout.
    fn libpng_color_type(&self) -> u8 {
        match self.pixels {
            Pixels::Rgb(_) => libpng_oracle::COLOR_RGB,
            Pixels::Rgba(_) => libpng_oracle::COLOR_RGBA,
        }
    }

    /// Encodes with gamut at the given knobs.
    fn gamut(&self, level: Level, filter: FilterStrategy, auto_reduce: bool) -> Vec<u8> {
        let encoder = PngEncoder::new()
            .with_compression(level)
            .with_filter(filter)
            .with_auto_reduce(auto_reduce);
        let dims = Dimensions::new(self.width, self.height).expect("corpus dimensions are valid");
        let mut out = Vec::new();
        match &self.pixels {
            Pixels::Rgb(v) => {
                let image = ImageRef::<Rgb8>::new(v, dims).expect("buffer matches dimensions");
                encoder.encode_image(image, &mut out).expect("encode");
            }
            Pixels::Rgba(v) => {
                let image = ImageRef::<Rgba8>::new(v, dims).expect("buffer matches dimensions");
                encoder.encode_image(image, &mut out).expect("encode");
            }
        }
        out
    }

    /// Encodes the *same source layout* with libpng at zlib level 9.
    ///
    /// Deliberately no `palette` option even for palettisable entries: handing libpng a palette
    /// would hand it gamut's own reduction, and the comparison would stop measuring anything.
    /// libpng's default adaptive filtering is left alone -- that is the honest baseline.
    fn libpng9(&self) -> Vec<u8> {
        let samples = match &self.pixels {
            Pixels::Rgb(v) | Pixels::Rgba(v) => v.as_slice(),
        };
        libpng_oracle::encode(
            samples,
            self.width,
            self.height,
            self.libpng_color_type(),
            8,
            &libpng_oracle::EncodeOpts {
                compression_level: Some(9),
                ..libpng_oracle::EncodeOpts::default()
            },
        )
    }
}

/// A deterministic, non-trivial RGB gradient -- the workspace's shared bench pattern. Avoids the
/// all-constant fast paths so the measured work reflects realistic entropy.
fn gradient_rgb(side: u32) -> Vec<u8> {
    let mut buf = vec![0u8; (side * side * 3) as usize];
    for y in 0..side {
        for x in 0..side {
            let i = ((y * side + x) * 3) as usize;
            buf[i] = (x ^ y) as u8;
            buf[i + 1] = x.wrapping_mul(3).wrapping_add(y) as u8;
            buf[i + 2] = x.wrapping_add(y.wrapping_mul(7)) as u8;
        }
    }
    buf
}

/// Smooth, photograph-like content: three integer sinusoid approximations at different periods.
/// Palette-hostile and 16-bit-hostile, so no reduction applies and the residual is the compressor
/// -- this is the row where gamut can lose to libpng, and the one to watch.
fn photo_rgb(side: u32) -> Vec<u8> {
    let mut buf = vec![0u8; (side * side * 3) as usize];
    for y in 0..side {
        for x in 0..side {
            let i = ((y * side + x) * 3) as usize;
            let (xi, yi) = (i64::from(x), i64::from(y));
            // Triangle waves stand in for sinusoids: smooth, periodic, no float in a fixture.
            let tri = |v: i64, period: i64| {
                let m = v.rem_euclid(period * 2);
                let up = if m < period { m } else { period * 2 - m };
                (up * 255 / period) as u8
            };
            buf[i] = tri(xi + yi, 61);
            buf[i + 1] = tri(xi * 2 - yi, 43);
            buf[i + 2] = tri(xi + yi * 3, 97);
        }
    }
    buf
}

/// Incompressible: a full avalanche mix of the byte index. Pins that the encoder does not
/// *expand* random data, and drives `FilterType::None`.
///
/// Deliberately not the plain `i * 2654435761 >> 24` the deflate bench uses. Over a dense index
/// that top byte changes only once every few hundred `i`, so the "noise" row compressed roughly
/// 97x and measured nothing at all. Three xorshift-multiply rounds give a byte that does not
/// correlate with its neighbours.
fn noise_rgb(side: u32) -> Vec<u8> {
    (0..(side * side * 3))
        .map(|i: u32| {
            let mut v = i.wrapping_add(0x9E37_79B9);
            v ^= v >> 16;
            v = v.wrapping_mul(0x21F0_AAAD);
            v ^= v >> 15;
            v = v.wrapping_mul(0x735A_2D97);
            v ^= v >> 15;
            v as u8
        })
        .collect()
}

/// Exactly 64 distinct colours over two alpha levels: the indexed + tRNS path, which is gamut's
/// single biggest structural lever over libpng-9 (libpng does not auto-palettise).
fn palette64_rgba(side: u32) -> Vec<u8> {
    let mut buf = vec![0u8; (side * side * 4) as usize];
    for y in 0..side {
        for x in 0..side {
            let i = ((y * side + x) * 4) as usize;
            let idx = ((x / 8 + y / 8 * 8) % 64) as u8;
            buf[i] = idx.wrapping_mul(4);
            buf[i + 1] = idx.wrapping_mul(9);
            buf[i + 2] = 255 - idx.wrapping_mul(3);
            buf[i + 3] = if idx.is_multiple_of(8) { 0 } else { 255 };
        }
    }
    buf
}

/// A sprite: binary alpha, and the fully transparent pixels carry *different* RGB values. That
/// invisible colour noise is what today's palette build keys on, so this is the only row that can
/// see the alpha-cleaning and tRNS-colour-key axes.
fn sprite_rgba(side: u32) -> Vec<u8> {
    let mut buf = vec![0u8; (side * side * 4) as usize];
    for y in 0..side {
        for x in 0..side {
            let i = ((y * side + x) * 4) as usize;
            let cx = i64::from(x) - i64::from(side) / 2;
            let cy = i64::from(y) - i64::from(side) / 2;
            let inside = cx * cx + cy * cy < (i64::from(side) * i64::from(side)) / 9;
            if inside {
                buf[i] = (x ^ y) as u8;
                buf[i + 1] = 0x40;
                buf[i + 2] = 0xC0;
                buf[i + 3] = 255;
            } else {
                // Invisible, and deliberately not constant.
                buf[i] = x as u8;
                buf[i + 1] = y as u8;
                buf[i + 2] = (x ^ y) as u8;
                buf[i + 3] = 0;
            }
        }
    }
    buf
}

/// One fully opaque colour: the compressible extreme, where the whole reduce cascade applies and
/// chunk framing is what is left to measure.
fn flat_rgba(side: u32) -> Vec<u8> {
    (0..(side * side))
        .flat_map(|_| [0x2E, 0x86, 0xC1, 0xFF])
        .collect()
}

/// A greyscale ramp presented as RGB: R=G=B everywhere, so the grey reduction applies.
fn grey_as_rgb(side: u32) -> Vec<u8> {
    let mut buf = vec![0u8; (side * side * 3) as usize];
    for y in 0..side {
        for x in 0..side {
            let i = ((y * side + x) * 3) as usize;
            let v = ((x + y) % 256) as u8;
            buf[i] = v;
            buf[i + 1] = v;
            buf[i + 2] = v;
        }
    }
    buf
}

/// The size-table corpus: one entry per axis that actually changes encoder behaviour.
fn corpus() -> Vec<Case> {
    let rgb = |name, pixels| Case {
        name,
        width: SIDE,
        height: SIDE,
        pixels: Pixels::Rgb(pixels),
    };
    let rgba = |name, pixels| Case {
        name,
        width: SIDE,
        height: SIDE,
        pixels: Pixels::Rgba(pixels),
    };
    vec![
        rgb("gradient_rgb8", gradient_rgb(SIDE)),
        rgb("photo_rgb8", photo_rgb(SIDE)),
        rgb("noise_rgb8", noise_rgb(SIDE)),
        rgb("grey_as_rgb8", grey_as_rgb(SIDE)),
        rgba("palette64_rgba8", palette64_rgba(SIDE)),
        rgba("sprite_rgba8", sprite_rgba(SIDE)),
        rgba("flat_rgba8", flat_rgba(SIDE)),
        // The regime where the signature and five chunks of framing dominate bits-per-pixel, and
        // the only row where `overhead_bytes` is legible.
        Case {
            name: "tiny_rgb8",
            width: 16,
            height: 16,
            pixels: Pixels::Rgb(gradient_rgb(16)),
        },
    ]
}

/// The knobs the size table reports gamut under: its default, and its smallest-output setting.
const BEST: (Level, FilterStrategy, bool) = (Level::Best, FilterStrategy::BruteForce, true);

/// Prints output size and bits-per-pixel against libpng at zlib level 9.
fn print_size_table() {
    println!(
        "\ngamut-png output size, bytes (lower is better); bpp is the whole file over the pixel count:\n\n\
         {:<17} {:>9} {:>9} {:>9} {:>9}  {:>9} {:>7} {:>7}",
        "input", "raw", "default", "best", "libpng-9", "best/lp9", "bpp", "lp9 bpp"
    );
    for case in corpus() {
        let default = case.gamut(Level::Default, FilterStrategy::MinSumAbs, false);
        let best = case.gamut(BEST.0, BEST.1, BEST.2);
        let libpng = case.libpng9();
        let delta = (best.len() as f64 / libpng.len().max(1) as f64 - 1.0) * 100.0;
        let bpp = |bytes: &[u8]| bytes.len() as f64 * 8.0 / f64::from(case.width * case.height);
        println!(
            "{:<17} {:>9} {:>9} {:>9} {:>9}  {:>8.1}% {:>7.3} {:>7.3}",
            case.name,
            case.raw_len(),
            default.len(),
            best.len(),
            libpng.len(),
            delta,
            bpp(&best),
            bpp(&libpng),
        );
    }
}

/// Prints where gamut's bytes went, stage by stage -- every column read back out of the encoded
/// file through [`deconstruct`], so the table describes the artefact rather than the encoder's
/// own bookkeeping.
fn print_stage_table() {
    println!(
        "\nwhere the bytes went (gamut at Level::Best + BruteForce + auto-reduce):\n\n\
         {:<17} {:>14} {:>5} {:>10} {:>10} {:>7} {:>9}  filters N/S/U/A/P",
        "input", "type", "depth", "filtered", "idat", "deflate", "overhead"
    );
    for case in corpus() {
        let png = case.gamut(BEST.0, BEST.1, BEST.2);
        let report = deconstruct(&png).expect("gamut's own output deconstructs");
        let filters = report.filters.map_or_else(
            || "-".to_string(),
            |h| {
                let n = |f| h.count(f);
                format!(
                    "{}/{}/{}/{}/{}",
                    n(FilterType::None),
                    n(FilterType::Sub),
                    n(FilterType::Up),
                    n(FilterType::Average),
                    n(FilterType::Paeth)
                )
            },
        );
        println!(
            "{:<17} {:>14} {:>5} {:>10} {:>10} {:>6.1}% {:>9}  {}",
            case.name,
            format!("{:?}", report.header.color_type),
            report.header.bit_depth,
            report.filtered_len,
            report.idat_compressed,
            report.idat_ratio() * 100.0,
            report.overhead_bytes(),
            filters,
        );
    }
}

fn case_named(name: &str) -> Case {
    corpus()
        .into_iter()
        .find(|c| c.name == name)
        .unwrap_or_else(|| panic!("unknown corpus entry {name}"))
}

#[divan::bench(args = [Level::Fast, Level::Default, Level::Best])]
fn encode_level(bencher: Bencher, level: Level) {
    let case = case_named("gradient_rgb8");
    bencher
        .counter(BytesCount::new(case.raw_len()))
        .bench_local(|| case.gamut(black_box(level), FilterStrategy::MinSumAbs, false));
}

#[divan::bench(args = [
    FilterStrategy::None,
    FilterStrategy::Fixed(FilterType::Paeth),
    FilterStrategy::MinSumAbs,
    FilterStrategy::BruteForce,
])]
fn encode_filter_strategy(bencher: Bencher, filter: FilterStrategy) {
    let case = case_named("gradient_rgb8");
    bencher
        .counter(BytesCount::new(case.raw_len()))
        .bench_local(|| case.gamut(Level::Default, black_box(filter), false));
}

/// Attributes the whole reduce stage without needing any seam into it: the same image encoded
/// with the analysis on and off.
#[divan::bench(args = [false, true])]
fn encode_auto_reduce(bencher: Bencher, auto_reduce: bool) {
    let case = case_named("palette64_rgba8");
    bencher
        .counter(BytesCount::new(case.raw_len()))
        .bench_local(|| {
            case.gamut(
                Level::Default,
                FilterStrategy::MinSumAbs,
                black_box(auto_reduce),
            )
        });
}

#[divan::bench(args = ["gradient_rgb8", "photo_rgb8", "noise_rgb8", "palette64_rgba8"])]
fn encode_corpus(bencher: Bencher, name: &str) {
    let case = case_named(name);
    bencher
        .counter(BytesCount::new(case.raw_len()))
        .bench_local(|| case.gamut(Level::Default, FilterStrategy::MinSumAbs, true));
}

/// Reading the accounting back out of a finished file -- the cost every table row pays.
#[divan::bench]
fn deconstruct_a_finished_png(bencher: Bencher) {
    let case = case_named("gradient_rgb8");
    let png = case.gamut(Level::Default, FilterStrategy::MinSumAbs, false);
    bencher
        .counter(BytesCount::new(png.len()))
        .bench_local(|| deconstruct(black_box(&png)).expect("deconstruct"));
}

/// Per-stage rows. Behind `test-support` because a `benches/` target is a separate crate and the
/// encoder's stages are crate-private; see `gamut_png::stages`.
#[cfg(feature = "test-support")]
mod stages {
    use gamut_png::stages;

    use super::{Bencher, BytesCount, Case, Pixels, SIDE, black_box, case_named, noise_rgb};

    /// The filtered stride and row length an RGB8 image of `SIDE` presents.
    const BPP: usize = 3;
    const ROW_BYTES: usize = SIDE as usize * BPP;

    fn rgb_samples(case: &Case) -> &[u8] {
        match &case.pixels {
            Pixels::Rgb(v) | Pixels::Rgba(v) => v,
        }
    }

    #[divan::bench(args = [
        gamut_png::FilterStrategy::None,
        gamut_png::FilterStrategy::Fixed(gamut_png::FilterType::Paeth),
        gamut_png::FilterStrategy::MinSumAbs,
    ])]
    fn filter_image(bencher: Bencher, strategy: gamut_png::FilterStrategy) {
        let case = case_named("gradient_rgb8");
        let samples = rgb_samples(&case).to_vec();
        bencher
            .counter(BytesCount::new(samples.len()))
            .bench_local(|| stages::filter_image(black_box(strategy), &samples, ROW_BYTES, BPP));
    }

    /// The per-scanline heuristic in isolation: five trial filterings plus five scorings, per row.
    #[divan::bench(args = [1usize, 3, 4])]
    fn choose_min_sum_abs(bencher: Bencher, bpp: usize) {
        let row: Vec<u8> = (0..ROW_BYTES).map(|i| (i * 7) as u8).collect();
        let prev: Vec<u8> = (0..ROW_BYTES).map(|i| (i * 13 + 5) as u8).collect();
        bencher
            .counter(BytesCount::new(row.len()))
            .with_inputs(Vec::new)
            .bench_local_refs(|scratch: &mut Vec<u8>| {
                stages::choose_min_sum_abs(&row, &prev, black_box(bpp), scratch)
            });
    }

    #[divan::bench(args = [1u8, 2, 4])]
    fn pack_scanlines(bencher: Bencher, depth: u8) {
        let samples = vec![1u8; (SIDE * SIDE) as usize];
        bencher
            .counter(BytesCount::new(samples.len()))
            .bench_local(|| {
                stages::pack_scanlines(&samples, SIDE as usize, SIDE as usize, black_box(depth))
            });
    }

    /// Both sides of the auto-reduce early exit: a palettisable image, and one with far more than
    /// 256 colours where the scan bails.
    #[divan::bench(args = ["palettisable", "too_many_colors"])]
    fn analyze8(bencher: Bencher, kind: &str) {
        let case = case_named(if kind == "palettisable" {
            "palette64_rgba8"
        } else {
            "photo_rgb8"
        });
        let channels = match case.pixels {
            Pixels::Rgb(_) => 3,
            Pixels::Rgba(_) => 4,
        };
        let samples = rgb_samples(&case).to_vec();
        bencher
            .counter(BytesCount::new(samples.len()))
            .bench_local(|| stages::analyze8(&samples, black_box(channels)));
    }

    /// 16-bit analysis, with and without a lawful demotion available: every sample `k * 257`
    /// demotes, an arbitrary one does not, and the two take different paths.
    #[divan::bench(args = [true, false])]
    fn analyze16(bencher: Bencher, demotable: bool) {
        let n = (SIDE * SIDE) as usize;
        let samples: Vec<u16> = (0..n)
            .map(|i| {
                let v = (i % 256) as u16;
                if demotable { v * 257 } else { v * 257 + 1 }
            })
            .collect();
        bencher
            .counter(BytesCount::new(samples.len() * 2))
            .bench_local(|| stages::analyze16(&samples, black_box(1)));
    }

    /// Runs over every IDAT byte, so it is on the critical path of every encode.
    #[divan::bench]
    fn crc32(bencher: Bencher) {
        let data = noise_rgb(SIDE);
        bencher
            .counter(BytesCount::new(data.len()))
            .bench_local(|| {
                let mut crc = stages::Crc32::new();
                crc.update(black_box(&data));
                crc.finish()
            });
    }
}
