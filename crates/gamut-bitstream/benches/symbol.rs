//! Throughput benchmark for the AV1 multi-symbol arithmetic (range) coder.
//!
//! The symbol coder is the entropy back-end for every coded tile, so its per-symbol cost dominates
//! the encoder's inner loop. This benches a long, fixed stream of symbols against a static skewed
//! CDF — the `disable_cdf_update = 1` path the M0 encoder actually uses. Run with
//! `cargo bench -p gamut-bitstream`.

use divan::counter::ItemsCount;
use gamut_bitstream::SymbolEncoder;

fn main() {
    divan::main();
}

/// Number of symbols encoded per timed iteration.
const N: usize = 4096;
/// Alphabet size of the benchmark CDF.
const NSYMS: usize = 8;

/// Small deterministic LCG so the symbol stream is reproducible without a `rand` dependency
/// (mirrors the generator in `symbol.rs`'s tests, which is not reachable from a bench target).
struct Lcg(u64);
impl Lcg {
    fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 32) as u32
    }
}

/// A fixed skewed cumulative CDF for `NSYMS` symbols (`cdf[last] == 32768`, strictly increasing).
fn skewed_cdf() -> [u16; NSYMS] {
    // Geometric-ish weights so the low symbols dominate, exercising the common skewed case.
    let weights = [40u32, 24, 14, 9, 5, 4, 2, 2];
    let total: u32 = weights.iter().sum();
    let mut cdf = [0u16; NSYMS];
    let mut acc = 0u32;
    for i in 0..NSYMS {
        acc += weights[i];
        cdf[i] = ((acc * 32768) / total) as u16;
    }
    cdf[NSYMS - 1] = 32768;
    cdf
}

#[divan::bench]
fn encode_symbols(bencher: divan::Bencher) {
    let cdf = skewed_cdf();
    let mut rng = Lcg(0x1234_5678_9abc_def0);
    let symbols: Vec<usize> = (0..N).map(|_| (rng.next_u32() as usize) % NSYMS).collect();
    bencher.counter(ItemsCount::new(N)).bench(|| {
        let mut enc = SymbolEncoder::new();
        for &s in &symbols {
            enc.encode_symbol(divan::black_box(s), &cdf);
        }
        enc.finish()
    });
}
