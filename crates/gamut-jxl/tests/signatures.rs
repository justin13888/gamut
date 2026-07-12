//! Byte-signature and `EncodeImage`-contract tests for the JPEG XL encoder: the two container
//! framings emit the correct magic, and encoding appends rather than replaces.
#![cfg(all(
    feature = "encode",
    any(not(target_arch = "wasm32"), target_os = "emscripten")
))]

use gamut_core::{Dimensions, EncodeImage, ImageRef, Rgb8};
use gamut_jxl::{Container, JxlEncoder};

/// A small deterministic RGB test image.
fn rgb_image(w: u32, h: u32) -> (Vec<u8>, Dimensions) {
    let dims = Dimensions::new(w, h).unwrap();
    let mut px = vec![0u8; (w * h * 3) as usize];
    for (i, b) in px.iter_mut().enumerate() {
        *b = (i * 31 + 7) as u8;
    }
    (px, dims)
}

#[test]
fn default_encoder_emits_bare_codestream_signature() {
    let (px, dims) = rgb_image(8, 8);
    let img = ImageRef::<Rgb8>::new(&px, dims).unwrap();
    let bytes = JxlEncoder::new().encode_to_vec(img).unwrap();
    // A bare JPEG XL codestream starts with the 2-byte signature 0xFF 0x0A.
    assert_eq!(&bytes[..2], &[0xFF, 0x0A], "codestream signature");
}

#[test]
fn container_encoder_emits_isobmff_box_signature() {
    let (px, dims) = rgb_image(8, 8);
    let img = ImageRef::<Rgb8>::new(&px, dims).unwrap();
    let bytes = JxlEncoder::new()
        .with_container(Container::IsoBmff)
        .encode_to_vec(img)
        .unwrap();
    // The ISO BMFF `.jxl` file starts with the 12-byte JXL signature box.
    const JXL_BOX: [u8; 12] = [
        0x00, 0x00, 0x00, 0x0C, 0x4A, 0x58, 0x4C, 0x20, 0x0D, 0x0A, 0x87, 0x0A,
    ];
    assert_eq!(&bytes[..12], &JXL_BOX, "ISO BMFF box signature");
}

#[test]
fn encode_image_appends_and_returns_bytes_written() {
    let (px, dims) = rgb_image(16, 9);
    let img = ImageRef::<Rgb8>::new(&px, dims).unwrap();

    // Pre-seed the buffer; the encoder must preserve it and append after it.
    let mut out = vec![0xDE, 0xAD, 0xBE, 0xEF];
    let written = JxlEncoder::new().encode_image(img, &mut out).unwrap();

    assert_eq!(&out[..4], &[0xDE, 0xAD, 0xBE, 0xEF], "prefix preserved");
    assert_eq!(written, out.len() - 4, "return value == bytes appended");
    // The appended region is itself a valid bare codestream.
    assert_eq!(&out[4..6], &[0xFF, 0x0A]);

    // encode_to_vec yields exactly the appended slice.
    let fresh = JxlEncoder::new().encode_to_vec(img).unwrap();
    assert_eq!(
        &out[4..],
        fresh.as_slice(),
        "appended bytes == fresh encode"
    );
}
