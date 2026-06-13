//! Encode-throughput benchmark for the M0 lossless AV1 intra-keyframe encoder.
//!
//! Sweeps square image sizes and reports throughput over the input RGB bytes, isolating the
//! encoder kernel (`encode_still_lossless_identity`) from the AVIF container — the end-to-end cost
//! is benched separately in `gamut-avif`. Run with `cargo bench -p gamut-av1`.

use divan::counter::BytesCount;
use gamut_av1::encode_still_lossless_identity;
use gamut_color::Planar8;

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
    let planes = Planar8::from_rgb8_identity(&rgb(side), side, side).unwrap();
    bencher
        .counter(BytesCount::new((side * side * 3) as usize))
        .bench(|| encode_still_lossless_identity(divan::black_box(&planes)).unwrap());
}
