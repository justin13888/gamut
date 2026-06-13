//! Throughput benchmarks for the AV1 lossless 4×4 Walsh–Hadamard transform pair.
//!
//! These are micro-benchmarks of the hot kernel that `gamut-av1` invokes once per 4×4 block of
//! every plane, so per-call cost compounds across an image. Run with `cargo bench -p gamut-dsp`.

use divan::counter::ItemsCount;
use gamut_dsp::{fwht4x4, iwht4x4};

fn main() {
    divan::main();
}

/// A deterministic 4×4 residual block spanning negative and positive samples.
fn block() -> [i32; 16] {
    std::array::from_fn(|i| i as i32 - 8)
}

#[divan::bench]
fn forward(bencher: divan::Bencher) {
    let input = block();
    bencher
        .counter(ItemsCount::new(input.len()))
        .bench(|| fwht4x4(divan::black_box(&input)));
}

#[divan::bench]
fn inverse(bencher: divan::Bencher) {
    let coeffs = fwht4x4(&block());
    bencher
        .counter(ItemsCount::new(coeffs.len()))
        .bench(|| iwht4x4(divan::black_box(&coeffs)));
}
