//! Bitstream / entropy-coding throughput benchmarks (issue #149).
//!
//! Intentionally tight: the three write-side primitives codecs spend real time in — MSB-first
//! bit packing, the AV1 range (symbol) coder, and LEB128 length coding. Each bench codes a
//! fixed-length stream and reports symbols (or values) emitted per second. Run with
//! `cargo bench -p gamut-bitstream`.

use divan::counter::ItemsCount;
use divan::{Bencher, black_box};
use gamut_bitstream::{BitWriter, SymbolEncoder, write_leb128};

fn main() {
    divan::main();
}

/// Number of symbols/values coded per measured iteration.
const N: usize = 4096;

#[divan::bench]
fn bitwriter_put_bits(bencher: Bencher) {
    // 11-bit values, a width that straddles byte boundaries on every write.
    let values: Vec<u32> = (0..N)
        .map(|i| (i as u32).wrapping_mul(2654435761) & 0x7FF)
        .collect();
    bencher.counter(ItemsCount::new(N)).bench_local(|| {
        let mut w = BitWriter::new();
        for &v in &values {
            w.put_bits(black_box(v), 11);
        }
        w.into_bytes()
    });
}

#[divan::bench]
fn symbol_encoder(bencher: Bencher) {
    // Uniform 8-symbol cumulative CDF (each step 4096; `cdf[7] == 32768`).
    let cdf: [u16; 8] = [4096, 8192, 12288, 16384, 20480, 24576, 28672, 32768];
    let symbols: Vec<usize> = (0..N).map(|i| (i * 7 + 3) % 8).collect();
    bencher.counter(ItemsCount::new(N)).bench_local(|| {
        let mut enc = SymbolEncoder::new();
        for &s in &symbols {
            enc.encode_symbol(black_box(s), &cdf);
        }
        enc.finish()
    });
}

#[divan::bench]
fn leb128_write(bencher: Bencher) {
    let values: Vec<u64> = (0..N)
        .map(|i| (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .collect();
    bencher.counter(ItemsCount::new(N)).bench_local(|| {
        let mut out = Vec::new();
        for &v in &values {
            write_leb128(&mut out, black_box(v));
        }
        out
    });
}
