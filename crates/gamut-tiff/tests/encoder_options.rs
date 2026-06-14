//! Encoder builder settings reach the output, and tile dimensions are validated (P10 / #110).

use gamut_core::{DecodeImage, Dimensions, EncodeImage, ImageBuf, ImageRef, Rgb8};
use gamut_tiff::{ByteOrder, Compression, Predictor, TiffDecoder, TiffEncoder};

fn rgb(w: u32, h: u32) -> Vec<u8> {
    (0..w * h * 3).map(|i| (i * 7) as u8).collect()
}

fn img<'a>(buf: &'a [u8], w: u32, h: u32) -> ImageRef<'a, Rgb8> {
    ImageRef::<Rgb8>::new(
        buf,
        Dimensions {
            width: w,
            height: h,
        },
    )
    .unwrap()
}

#[test]
fn with_byte_order_writes_requested_endianness() {
    let buf = rgb(8, 8);
    let mut be = Vec::new();
    TiffEncoder::new()
        .with_byte_order(ByteOrder::BigEndian)
        .encode_image(img(&buf, 8, 8), &mut be)
        .unwrap();
    assert_eq!(&be[0..2], b"MM", "big-endian header");
    let mut le = Vec::new();
    TiffEncoder::new()
        .with_byte_order(ByteOrder::LittleEndian)
        .encode_image(img(&buf, 8, 8), &mut le)
        .unwrap();
    assert_eq!(&le[0..2], b"II", "little-endian header");
}

#[test]
fn with_predictor_is_applied_and_preserves_chained_settings() {
    let buf = rgb(16, 16);
    // Chaining with_predictor after with_byte_order(BE) must keep BE — a builder that returned
    // `Default::default()` would reset it — and the predictor must round-trip through the decoder.
    let mut tiff = Vec::new();
    TiffEncoder::new()
        .with_byte_order(ByteOrder::BigEndian)
        .with_compression(Compression::Lzw)
        .with_predictor(Predictor::HorizontalDifferencing)
        .encode_image(img(&buf, 16, 16), &mut tiff)
        .unwrap();
    assert_eq!(
        &tiff[0..2],
        b"MM",
        "with_predictor kept the big-endian setting"
    );
    let back: ImageBuf<Rgb8> = TiffDecoder::new().decode_image(&tiff).unwrap();
    assert_eq!(back.as_samples(), buf.as_slice(), "predictor round-trips");
}

#[test]
fn tile_dimensions_must_be_positive_multiples_of_16() {
    let buf = rgb(32, 32);
    // Each invalid dimension is rejected on its own, pinning every disjunct of the validation
    // (`tw == 0 || th == 0 || tw % 16 != 0 || th % 16 != 0`).
    for (tw, th) in [
        (0u32, 16u32),
        (16, 0),
        (15, 16),
        (16, 15),
        (16, 17),
        (17, 16),
    ] {
        let mut out = Vec::new();
        let r = TiffEncoder::new()
            .with_tiling(tw, th)
            .encode_image(img(&buf, 32, 32), &mut out);
        assert!(r.is_err(), "tiling {tw}x{th} must be rejected");
    }
    // The all-valid case still succeeds (so the checks aren't simply always-erroring).
    let mut out = Vec::new();
    TiffEncoder::new()
        .with_tiling(16, 16)
        .encode_image(img(&buf, 32, 32), &mut out)
        .expect("16x16 tiles are valid");
}
