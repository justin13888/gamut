//! Feature-grid differential matrix: for a spread of container framings, effort levels and pixel
//! layouts, gamut encodes with libjxl and both independent decoders — gamut's pure-Rust
//! [`JxlDecoder`] (jxl-rs) and the reference libjxl oracle — must reconstruct the source **bit-for-bit**
//! on a lossless stream.
//!
//! This complements `oracle.rs` (which sweeps sizes and pins the lossy bounds / effort-differ /
//! animation-rejection behaviours) by sweeping the *encoder configuration* grid instead, and adds the
//! sequential `decode_image_into` reuse-after-replacement scenario. Behaviours already pinned in
//! `oracle.rs`/`roundtrip.rs` are referenced there, not duplicated here.
//!
//! Uses both codec halves and the libjxl oracle, so it is compiled only when both are available.
#![cfg(all(
    feature = "encode",
    feature = "decode",
    any(not(target_arch = "wasm32"), target_os = "emscripten")
))]

mod common;

use common::{DecodedSamples, decode, gen_u8, gen_u16};
use gamut_core::{
    DecodeImage, Dimensions, EncodeImage, Gray8, ImageBuf, ImageRef, Pixel, Rgb16, Rgba8,
};
use gamut_jxl::{Container, Effort, JxlDecoder, JxlEncoder};

/// The grid's fixed textured size: non-square, both dimensions odd, larger than a DCT group edge so
/// effort actually has something to chew on.
const W: u32 = 33;
const H: u32 = 29;

/// The container framings swept.
const CONTAINERS: [Container; 2] = [Container::Codestream, Container::IsoBmff];

/// The effort levels swept: the two extremes plus the default.
const EFFORTS: [Effort; 3] = [Effort::Lightning, Effort::Squirrel, Effort::Glacier];

/// Emits a full-grid lossless differential test for one 8-bit pixel layout: across every
/// container × effort cell, gamut and the libjxl oracle both decode bit-exact to the source.
macro_rules! grid_u8 {
    ($name:ident, $pixel:ty) => {
        #[test]
        fn $name() {
            let ch = <$pixel as Pixel>::CHANNELS;
            let px = gen_u8(W, H, ch);
            let dims = Dimensions::new(W, H).unwrap();
            let img = ImageRef::<$pixel>::new(&px, dims).unwrap();

            for container in CONTAINERS {
                for effort in EFFORTS {
                    let bytes = JxlEncoder::lossless()
                        .with_container(container)
                        .with_effort(effort)
                        .encode_to_vec(img)
                        .unwrap();

                    // gamut's decoder (jxl-rs).
                    let gamut: ImageBuf<$pixel> = JxlDecoder::new().decode_image(&bytes).unwrap();
                    assert_eq!(gamut.dimensions(), dims, "{container:?}/{effort:?} dims");
                    assert_eq!(
                        gamut.as_samples(),
                        px.as_slice(),
                        "gamut != source {container:?}/{effort:?}"
                    );

                    // The libjxl oracle.
                    let oracle = decode(&bytes);
                    assert_eq!((oracle.width, oracle.height), (W, H));
                    assert_eq!(oracle.num_channels as usize, ch);
                    let DecodedSamples::U8(oracle) = oracle.samples else {
                        panic!("oracle produced non-u8 samples {container:?}/{effort:?}");
                    };
                    assert_eq!(oracle, px, "oracle != source {container:?}/{effort:?}");
                }
            }
        }
    };
}

/// Emits a full-grid lossless differential test for one 16-bit pixel layout.
macro_rules! grid_u16 {
    ($name:ident, $pixel:ty) => {
        #[test]
        fn $name() {
            let ch = <$pixel as Pixel>::CHANNELS;
            let px = gen_u16(W, H, ch);
            let dims = Dimensions::new(W, H).unwrap();
            let img = ImageRef::<$pixel>::new(&px, dims).unwrap();

            for container in CONTAINERS {
                for effort in EFFORTS {
                    let bytes = JxlEncoder::lossless()
                        .with_container(container)
                        .with_effort(effort)
                        .encode_to_vec(img)
                        .unwrap();

                    let gamut: ImageBuf<$pixel> = JxlDecoder::new().decode_image(&bytes).unwrap();
                    assert_eq!(gamut.dimensions(), dims, "{container:?}/{effort:?} dims");
                    assert_eq!(
                        gamut.as_samples(),
                        px.as_slice(),
                        "gamut != source {container:?}/{effort:?}"
                    );

                    let oracle = decode(&bytes);
                    assert_eq!((oracle.width, oracle.height), (W, H));
                    assert_eq!(oracle.num_channels as usize, ch);
                    let DecodedSamples::U16(oracle) = oracle.samples else {
                        panic!("oracle produced non-u16 samples {container:?}/{effort:?}");
                    };
                    assert_eq!(oracle, px, "oracle != source {container:?}/{effort:?}");
                }
            }
        }
    };
}

// The three grids span the axes that matter: bit width (8/16), colour family (gray/RGB), and alpha
// presence — Gray8 (1×u8), Rgba8 (4×u8) and Rgb16 (3×u16).
grid_u8!(feature_grid_gray8, Gray8);
grid_u8!(feature_grid_rgba8, Rgba8);
grid_u16!(feature_grid_rgb16, Rgb16);

#[test]
fn decode_into_reuses_allocation_after_a_replacement() {
    // A single destination taken through both `decode_image_into` paths in sequence: first a
    // dimension *mismatch* (8×8 dst, 16×16 image) that must replace the buffer, then a *match*
    // (another 16×16 image) that must reuse the freshly-sized allocation in place. This pins the
    // reuse contract across a prior replacement — a scenario neither single-shot test in `roundtrip.rs`
    // covers.
    let dims = Dimensions::new(16, 16).unwrap();
    let px_a = gen_u8(16, 16, Rgba8::CHANNELS);
    let bytes_a = JxlEncoder::lossless()
        .encode_to_vec(ImageRef::<Rgba8>::new(&px_a, dims).unwrap())
        .unwrap();

    // A second, distinct 16×16 image so the reuse step is a real decode, not a repeat.
    let mut px_b = px_a.clone();
    for b in &mut px_b {
        *b = b.wrapping_add(37);
    }
    let bytes_b = JxlEncoder::lossless()
        .encode_to_vec(ImageRef::<Rgba8>::new(&px_b, dims).unwrap())
        .unwrap();

    // Start with a mismatched (8×8) destination: the first decode must resize it to 16×16.
    let mut dst = ImageBuf::<Rgba8>::zeroed(Dimensions::new(8, 8).unwrap()).unwrap();
    JxlDecoder::new()
        .decode_image_into(&bytes_a, &mut dst)
        .unwrap();
    assert_eq!(dst.dimensions(), dims, "dst resized to the decoded dims");
    assert_eq!(dst.as_samples(), px_a.as_slice(), "first decode correct");

    // Now the geometry matches, so the second decode must reuse the same sample allocation.
    let ptr_before = dst.as_samples().as_ptr();
    JxlDecoder::new()
        .decode_image_into(&bytes_b, &mut dst)
        .unwrap();
    assert_eq!(
        dst.as_samples().as_ptr(),
        ptr_before,
        "allocation reused in place"
    );
    assert_eq!(dst.as_samples(), px_b.as_slice(), "second decode correct");
}

#[test]
fn multi_chunk_output_drain_roundtrips_bit_exact() {
    // A high-entropy image that lossless coding cannot shrink below the encoder's 64 KiB initial
    // output chunk, so libjxl's `ProcessOutput` must be drained over several growing chunks. This
    // exercises the encoder's multi-iteration output-growth loop (the `NEED_MORE_OUTPUT` arm, unused
    // by the small images elsewhere) and confirms the reassembled stream is still a bit-exact
    // lossless round-trip — i.e. the chunk stitching preserves every byte.
    let (w, h) = (256u32, 256u32);
    let mut px = vec![0u8; (w * h * 4) as usize];
    let mut state = 0x0001_2345u32;
    for b in &mut px {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *b = (state >> 24) as u8;
    }
    let dims = Dimensions::new(w, h).unwrap();
    let img = ImageRef::<Rgba8>::new(&px, dims).unwrap();
    let bytes = JxlEncoder::lossless().encode_to_vec(img).unwrap();
    assert!(
        bytes.len() > 64 * 1024,
        "high-entropy stream ({} bytes) must exceed the 64 KiB first chunk to drive the growth loop",
        bytes.len()
    );

    let out: ImageBuf<Rgba8> = JxlDecoder::new().decode_image(&bytes).unwrap();
    assert_eq!(out.dimensions(), dims);
    assert_eq!(
        out.as_samples(),
        px.as_slice(),
        "multi-chunk output must round-trip bit-exact"
    );
}
