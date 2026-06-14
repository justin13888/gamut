//! AVIF encode throughput benchmarks (issue #149).
//!
//! Intentionally tight: a `qindex` sweep over the full encode-and-wrap path (AV1 intra frame
//! plus ISOBMFF container assembly), with `qindex = 0` exercising the lossless path and the
//! rest the lossy speed/quality tradeoff. Counters report source-pixel bytes per second. The
//! AV1 intra core dominates, so the image is small. Run with `cargo bench -p gamut-avif`.

use divan::counter::BytesCount;
use divan::{Bencher, black_box};
use gamut_avif::AvifEncoder;
use gamut_core::{Dimensions, EncodeImage, ImageRef, Rgb8};

fn main() {
    divan::main();
}

/// Side length of the square RGB test image. Small: AVIF wraps the heavy AV1 intra encoder.
const SIDE: u32 = 128;

/// A deterministic, non-trivial RGB gradient so the encoder sees realistic residual energy.
fn gradient_rgb() -> Vec<u8> {
    let side = SIDE;
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

/// `qindex = 0` is lossless; the larger values walk the lossy quantizer (speed/quality) curve.
#[divan::bench(args = [0, 64, 128, 200])]
fn encode(bencher: Bencher, qindex: u8) {
    let rgb = gradient_rgb();
    let dims = Dimensions {
        width: SIDE,
        height: SIDE,
    };
    bencher
        .counter(BytesCount::new((SIDE * SIDE * 3) as usize))
        .bench_local(|| {
            let mut out = Vec::new();
            let image = ImageRef::<Rgb8>::new(black_box(&rgb), dims).unwrap();
            AvifEncoder::new()
                .with_qindex(qindex)
                .encode_image(image, &mut out)
                .unwrap();
            out
        });
}
