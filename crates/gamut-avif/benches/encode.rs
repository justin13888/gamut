//! End-to-end encode-throughput benchmark: 8-bit RGB → AVIF still image.
//!
//! Measures the full `AvifEncoder::encode_rgb8` path (identity-plane conversion + AV1 encode +
//! ISOBMFF container), including a fresh output allocation per call to reflect realistic caller
//! cost. Throughput is reported over the input RGB bytes. Run with `cargo bench -p gamut-avif`.

use divan::counter::BytesCount;
use gamut_avif::AvifEncoder;
use gamut_core::Dimensions;

fn main() {
    divan::main();
}

/// Side lengths of the square test images (small / medium / large).
const SIZES: &[u32] = &[64, 256, 512];

/// Builds a deterministic interleaved-RGB buffer with per-pixel variation so the encoder does real
/// residual work rather than coding a flat block.
fn rgb(side: u32) -> Vec<u8> {
    let mut buf = vec![0u8; (side * side * 3) as usize];
    for y in 0..side {
        for x in 0..side {
            let i = ((y * side + x) * 3) as usize;
            buf[i] = (x.wrapping_mul(7).wrapping_add(y.wrapping_mul(3))) as u8;
            buf[i + 1] = (x.wrapping_mul(x).wrapping_add(y)) as u8;
            buf[i + 2] = (x ^ y.wrapping_mul(5)) as u8;
        }
    }
    buf
}

#[divan::bench(args = SIZES)]
fn encode(bencher: divan::Bencher, side: u32) {
    let pixels = rgb(side);
    let encoder = AvifEncoder::new();
    let dims = Dimensions {
        width: side,
        height: side,
    };
    bencher.counter(BytesCount::new(pixels.len())).bench(|| {
        let mut out = Vec::new();
        encoder
            .encode_rgb8(divan::black_box(&pixels), dims, &mut out)
            .unwrap();
        out
    });
}
