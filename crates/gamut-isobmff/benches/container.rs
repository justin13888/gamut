//! ISOBMFF (AVIF) container serialization throughput benchmarks (issue #149).
//!
//! Intentionally tight: the `write_avif_still` box assembler — `ftyp`/`meta`/`mdat` layout with
//! `iloc` offset back-patching — measured at a small and a large payload so the fixed box
//! overhead and the payload copy are both visible. The counter reports payload bytes per second
//! (`item_data` is synthetic; the writer never parses it). Run with `cargo bench -p gamut-isobmff`.

use divan::counter::BytesCount;
use divan::{Bencher, black_box};
use gamut_isobmff::{Av1cConfig, AvifStillImage, ImageTransform, NclxColr, write_avif_still};

fn main() {
    divan::main();
}

#[divan::bench(args = [4 * 1024, 256 * 1024])]
fn write_still(bencher: Bencher, payload: usize) {
    let item = vec![0xA5u8; payload];
    let img = AvifStillImage {
        width: 1024,
        height: 1024,
        bit_depth: 8,
        num_channels: 3,
        av1c: Av1cConfig {
            seq_profile: 0,
            seq_level_idx_0: 1,
            seq_tier_0: 0,
            high_bitdepth: false,
            twelve_bit: false,
            monochrome: false,
            chroma_subsampling_x: 0,
            chroma_subsampling_y: 0,
            chroma_sample_position: 0,
        },
        nclx: NclxColr {
            colour_primaries: 1,
            transfer_characteristics: 13,
            matrix_coefficients: 0,
            full_range: true,
        },
        transform: ImageTransform::default(),
        item_data: &item,
    };
    bencher
        .counter(BytesCount::new(payload))
        .bench_local(|| write_avif_still(black_box(&img)));
}
