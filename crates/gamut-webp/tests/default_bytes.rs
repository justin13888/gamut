//! Byte-identical-default regression: with no backend pushed, every encode path must produce
//! exactly the bytes the built-in `vp8`/`vp8l` tails produced before the backend registries landed
//! (issue #275), and every decode path must produce exactly the same pixels.
//!
//! The fixtures are pinned by length + FNV-1a-64 digest (and, for the smallest case, the full byte
//! vector), so any change to the default output — including one that merely re-orders chunks — fails
//! here rather than silently shipping. The digests were captured from the pre-change encoder.

use gamut_core::convert::{AlphaPolicy, ConvertPolicy};
use gamut_core::{DecodeImage, Dimensions, EncodeImage, ImageBuf, ImageRef, Rgb8, Rgba8};
use gamut_webp::{WebpDecoder, WebpEncoder};

/// FNV-1a (64-bit) over `bytes` — a dependency-free digest for pinning fixture bytes.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn dims(width: u32, height: u32) -> Dimensions {
    Dimensions { width, height }
}

/// A deterministic RGB gradient.
fn rgb_fixture(w: u32, h: u32) -> Vec<u8> {
    (0..w * h)
        .flat_map(|i| {
            let (x, y) = (i % w, i / w);
            [(x * 7) as u8, (y * 11) as u8, (x ^ y) as u8]
        })
        .collect()
}

/// A deterministic RGBA gradient with non-trivial alpha.
fn rgba_fixture(w: u32, h: u32) -> Vec<u8> {
    (0..w * h)
        .flat_map(|i| {
            let (x, y) = (i % w, i / w);
            [
                (x * 7) as u8,
                (y * 11) as u8,
                (x ^ y) as u8,
                ((x * 5 + y * 3) & 0xff) as u8,
            ]
        })
        .collect()
}

fn encode_rgb(enc: &WebpEncoder, px: &[u8], d: Dimensions) -> Vec<u8> {
    let mut out = Vec::new();
    enc.encode_image(ImageRef::<Rgb8>::new(px, d).expect("rgb fixture"), &mut out)
        .expect("encode");
    out
}

fn encode_rgba(enc: &WebpEncoder, px: &[u8], d: Dimensions) -> Vec<u8> {
    let mut out = Vec::new();
    enc.encode_image(
        ImageRef::<Rgba8>::new(px, d).expect("rgba fixture"),
        &mut out,
    )
    .expect("encode");
    out
}

/// Asserts the exact length and digest of a fixture's bytes.
fn assert_bytes(what: &str, bytes: &[u8], len: usize, digest: u64) {
    assert_eq!(bytes.len(), len, "{what}: length changed");
    assert_eq!(fnv1a64(bytes), digest, "{what}: bytes changed");
}

#[test]
fn lossless_rgb_default_bytes_are_unchanged() {
    let file = encode_rgb(&WebpEncoder::lossless(), &rgb_fixture(24, 16), dims(24, 16));
    assert_bytes("lossless rgb", &file, 696, 0x681b_82d9_857c_7277);
}

#[test]
fn lossless_rgba_default_bytes_are_unchanged() {
    let file = encode_rgba(
        &WebpEncoder::lossless(),
        &rgba_fixture(24, 16),
        dims(24, 16),
    );
    assert_bytes("lossless rgba", &file, 690, 0x9a2f_8f5b_407c_73ae);
}

#[test]
fn lossy_rgb_default_bytes_are_unchanged() {
    let file = encode_rgb(&WebpEncoder::lossy(60), &rgb_fixture(32, 24), dims(32, 24));
    assert_bytes("lossy rgb", &file, 136, 0x2eb3_c30d_36b6_067e);
}

#[test]
fn lossy_rgba_extended_default_bytes_are_unchanged() {
    // The transparent path: VP8X + ALPH + VP8 . `ALPH` stays container-side, so its bytes are
    // pinned here too.
    let file = encode_rgba(&WebpEncoder::lossy(60), &rgba_fixture(32, 24), dims(32, 24));
    assert_bytes("lossy rgba extended", &file, 932, 0xbb55_353e_7974_f6ea);
}

#[test]
fn lossy_rgba_opaque_default_bytes_are_unchanged() {
    // Small enough to pin every byte of the file, container header included.
    const OPAQUE: [u8; 58] = [
        0x52, 0x49, 0x46, 0x46, 0x32, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50, 0x38,
        0x20, 0x26, 0x00, 0x00, 0x00, 0x70, 0x01, 0x00, 0x9d, 0x01, 0x2a, 0x10, 0x00, 0x10, 0x00,
        0x06, 0x40, 0x64, 0x08, 0x00, 0x2b, 0x82, 0x64, 0x34, 0xc7, 0xdc, 0xfe, 0x6d, 0x4c, 0x7f,
        0xff, 0x57, 0x7e, 0x76, 0xfd, 0x4f, 0x7b, 0x1f, 0xc1, 0x96, 0xdb, 0x04, 0x00,
    ];
    let px = [120u8, 60, 200, 0xff].repeat(16 * 16);
    let file = encode_rgba(&WebpEncoder::lossy(60), &px, dims(16, 16));
    assert_eq!(file.as_slice(), OPAQUE.as_slice());
}

#[test]
fn default_decode_output_is_unchanged() {
    let lossy = encode_rgb(&WebpEncoder::lossy(60), &rgb_fixture(32, 24), dims(32, 24));
    let rgb: ImageBuf<Rgb8> = WebpDecoder::new().decode_image(&lossy).expect("decode");
    assert_eq!(rgb.dimensions(), dims(32, 24));
    assert_bytes(
        "lossy decode rgb",
        rgb.as_samples(),
        2304,
        0xb02e_c3fd_9b13_11be,
    );

    let ext = encode_rgba(&WebpEncoder::lossy(60), &rgba_fixture(32, 24), dims(32, 24));
    let rgba: ImageBuf<Rgba8> = WebpDecoder::new().decode_image(&ext).expect("decode");
    assert_eq!(rgba.dimensions(), dims(32, 24));
    assert_bytes(
        "extended decode rgba",
        rgba.as_samples(),
        3072,
        0x6c6d_e0f3_01d5_355a,
    );
}

/// The policy set by `convert_policy` must reach the typed decode.
///
/// A file that genuinely carries transparency cannot be presented as RGB without discarding it, so
/// the default decoder refuses; naming an `AlphaPolicy` permits it. A decoder that dropped the
/// setter would refuse both times.
#[test]
fn convert_policy_reaches_the_typed_decode() {
    let dims = Dimensions {
        width: 2,
        height: 2,
    };
    // Alpha varies, so the file is genuinely transparent rather than opaque-and-therefore-lossless.
    #[rustfmt::skip]
    let rgba = [
        0x10u8, 0x20, 0x30, 0x00,
        0x40,   0x50, 0x60, 0x80,
        0x70,   0x80, 0x90, 0xc0,
        0xa0,   0xb0, 0xc0, 0xff,
    ];
    let webp = WebpEncoder::lossless()
        .encode_to_vec(ImageRef::<Rgba8>::new(&rgba, dims).unwrap())
        .expect("encode");

    let refused = DecodeImage::<Rgb8>::decode_image(&WebpDecoder::new(), &webp)
        .expect_err("alpha must not be discarded silently");
    assert_eq!(refused.kind(), gamut_core::ErrorKind::Unsupported);

    let dropped: ImageBuf<Rgb8> = WebpDecoder::new()
        .convert_policy(ConvertPolicy::lossless().with_alpha(AlphaPolicy::Drop))
        .decode_image(&webp)
        .expect("drop decode");
    let expected: Vec<u8> = rgba
        .chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect();
    assert_eq!(dropped.as_samples(), expected.as_slice());

    // Compositing is a different answer to the same question, so it must not agree with dropping
    // on the pixels that are actually transparent.
    let composited: ImageBuf<Rgb8> = WebpDecoder::new()
        .convert_policy(
            ConvertPolicy::lossless()
                .with_alpha(AlphaPolicy::CompositeOver)
                .with_background([u16::MAX; 3]),
        )
        .decode_image(&webp)
        .expect("composite decode");
    assert_eq!(&composited.as_samples()[0..3], &[255, 255, 255]);
    assert_ne!(composited.as_samples(), dropped.as_samples());
}
