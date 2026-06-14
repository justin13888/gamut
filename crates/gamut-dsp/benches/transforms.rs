//! DSP transform throughput benchmarks (issue #149).
//!
//! Intentionally tight: the forward/inverse 1-D transform kernels codecs invoke per block —
//! DCT and ADST across their supported block sizes, plus the 4×4 Walsh–Hadamard transform used
//! for lossless residuals. Counters report coefficients processed per second. The `n` argument
//! is the transform's size exponent: length `= 1 << n`. Run with `cargo bench -p gamut-dsp`.

use divan::counter::ItemsCount;
use divan::{Bencher, black_box};
use gamut_dsp::{forward_adst, forward_dct, fwht4x4, inverse_adst, inverse_dct, iwht4x4};

fn main() {
    divan::main();
}

/// Intermediate clamping range passed to the inverse transforms (matches the unit tests).
const RANGE: u32 = 16;

/// Deterministic, bounded coefficients — small enough to stay clear of the transforms'
/// intermediate-overflow limits while still exercising every butterfly.
fn coeffs(len: usize) -> Vec<i64> {
    (0..len)
        .map(|i| ((i as i64 * 73 + 11) % 511) - 255)
        .collect()
}

#[divan::bench(args = [2, 3, 4, 5, 6])]
fn forward_dct_n(bencher: Bencher, n: u32) {
    let len = 1usize << n;
    bencher
        .counter(ItemsCount::new(len))
        .with_inputs(|| coeffs(len))
        .bench_local_refs(|t| forward_dct(t, n));
}

#[divan::bench(args = [2, 3, 4, 5, 6])]
fn inverse_dct_n(bencher: Bencher, n: u32) {
    let len = 1usize << n;
    bencher
        .counter(ItemsCount::new(len))
        .with_inputs(|| coeffs(len))
        .bench_local_refs(|t| inverse_dct(t, n, RANGE));
}

#[divan::bench(args = [2, 3, 4])]
fn forward_adst_n(bencher: Bencher, n: u32) {
    let len = 1usize << n;
    bencher
        .counter(ItemsCount::new(len))
        .with_inputs(|| coeffs(len))
        .bench_local_refs(|t| forward_adst(t, n));
}

#[divan::bench(args = [2, 3, 4])]
fn inverse_adst_n(bencher: Bencher, n: u32) {
    let len = 1usize << n;
    bencher
        .counter(ItemsCount::new(len))
        .with_inputs(|| coeffs(len))
        .bench_local_refs(|t| inverse_adst(t, n, RANGE));
}

#[divan::bench]
fn forward_wht_4x4(bencher: Bencher) {
    let mut block = [0i32; 16];
    for (i, v) in block.iter_mut().enumerate() {
        *v = ((i as i32 * 37 + 5) % 255) - 127;
    }
    bencher
        .counter(ItemsCount::new(16usize))
        .bench_local(|| fwht4x4(black_box(&block)));
}

#[divan::bench]
fn inverse_wht_4x4(bencher: Bencher) {
    let mut residual = [0i32; 16];
    for (i, v) in residual.iter_mut().enumerate() {
        *v = ((i as i32 * 37 + 5) % 255) - 127;
    }
    let quant = fwht4x4(&residual);
    bencher
        .counter(ItemsCount::new(16usize))
        .bench_local(|| iwht4x4(black_box(&quant)));
}
