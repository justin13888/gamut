//! Public decoder resource-limit contract: exact boundaries, native byte accounting, and hostile
//! frame headers rejected before a frame-sized allocation is possible.

use gamut_core::{DecodeImage, Dimensions, EncodeImage, Gray8, ImageBuf, ImageRef, Rgb8};
use gamut_jpeg::{JpegDecoder, JpegEncoder};

fn gray_jpeg(width: u32, height: u32) -> Vec<u8> {
    let pixels = vec![127; (width * height) as usize];
    JpegEncoder::new()
        .encode_to_vec(
            ImageRef::<Gray8>::new(&pixels, Dimensions::new(width, height).unwrap()).unwrap(),
        )
        .unwrap()
}

fn rgb_jpeg(width: u32, height: u32) -> Vec<u8> {
    let pixels = vec![127; (width * height * 3) as usize];
    JpegEncoder::new()
        .encode_to_vec(
            ImageRef::<Rgb8>::new(&pixels, Dimensions::new(width, height).unwrap()).unwrap(),
        )
        .unwrap()
}

fn gray_error(decoder: &JpegDecoder, jpeg: &[u8]) -> String {
    DecodeImage::<Gray8>::decode_image(decoder, jpeg)
        .unwrap_err()
        .to_string()
}

#[test]
fn dimension_limits_are_exact_and_independent() {
    let jpeg = gray_jpeg(17, 13);
    let exact: ImageBuf<Gray8> = JpegDecoder::new()
        .with_max_dimensions(17, 13)
        .decode_image(&jpeg)
        .unwrap();
    assert_eq!(exact.dimensions(), Dimensions::new(17, 13).unwrap());

    assert_eq!(
        gray_error(&JpegDecoder::new().with_max_dimensions(16, 13), &jpeg),
        "unsupported: JPEG: image exceeds the dimension limit"
    );
    assert_eq!(
        gray_error(&JpegDecoder::new().with_max_dimensions(17, 12), &jpeg),
        "unsupported: JPEG: image exceeds the dimension limit"
    );
}

#[test]
fn byte_limit_uses_the_frame_component_count_at_an_exact_boundary() {
    let gray = gray_jpeg(8, 8);
    assert!(
        DecodeImage::<Gray8>::decode_image(&JpegDecoder::new().with_max_image_bytes(8 * 8), &gray)
            .is_ok()
    );
    assert_eq!(
        gray_error(&JpegDecoder::new().with_max_image_bytes(8 * 8 - 1), &gray),
        "unsupported: JPEG: image exceeds the size limit"
    );

    // The encoder's default 4:2:0 subsampling does not discount the native three-channel raster.
    let rgb = rgb_jpeg(8, 8);
    assert!(
        DecodeImage::<Rgb8>::decode_image(
            &JpegDecoder::new().with_max_image_bytes(8 * 8 * 3),
            &rgb
        )
        .is_ok()
    );
    assert_eq!(
        DecodeImage::<Rgb8>::decode_image(
            &JpegDecoder::new().with_max_image_bytes(8 * 8 * 3 - 1),
            &rgb
        )
        .unwrap_err()
        .to_string(),
        "unsupported: JPEG: image exceeds the size limit"
    );
}

#[test]
fn synthetic_pathological_sof_is_rejected_by_the_cap_first() {
    // SOI + one-component SOF0 declaring 65535×65535 + EOI. There are deliberately no tables or
    // scan: the limit error proves geometry is refused at the SOF, before later decode state matters.
    let jpeg = [
        0xFF, 0xD8, // SOI
        0xFF, 0xC0, 0, 11, // SOF0, length 11
        8, 0xFF, 0xFF, 0xFF, 0xFF, // P=8, Y=65535, X=65535
        1, 1, 0x11, 0, // one component
        0xFF, 0xD9, // EOI
    ];
    assert_eq!(
        gray_error(&JpegDecoder::new().with_max_dimensions(4096, 4096), &jpeg),
        "unsupported: JPEG: image exceeds the dimension limit"
    );
    assert_eq!(
        gray_error(&JpegDecoder::new().with_max_image_bytes(64 << 20), &jpeg),
        "unsupported: JPEG: image exceeds the size limit"
    );
}

#[test]
fn limits_are_cloneable_and_defaults_remain_unrestricted() {
    let jpeg = gray_jpeg(8, 8);
    let default: ImageBuf<Gray8> = JpegDecoder::default().decode_image(&jpeg).unwrap();
    assert_eq!(default.dimensions(), Dimensions::new(8, 8).unwrap());

    let limited = JpegDecoder::new().with_max_dimensions(7, 8).clone();
    assert_eq!(
        gray_error(&limited, &jpeg),
        "unsupported: JPEG: image exceeds the dimension limit"
    );
}
