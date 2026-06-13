//! A DNG is a valid TIFF, so an independent TIFF reader (**libtiff**) must parse our container and
//! decode IFD 0 (the RGB preview). This cross-checks the byte-order header, IFD layout, and the
//! preview strip offsets/data without going through the Adobe SDK.

mod common;

use gamut_dng::{ByteOrder, DngEncoder};

#[test]
fn libtiff_reads_ifd0_preview() {
    for &(order, w, h) in &[
        (ByteOrder::LittleEndian, 64u32, 48u32),
        (ByteOrder::BigEndian, 40, 30),
        (ByteOrder::LittleEndian, 33, 21), // odd dimensions
    ] {
        let raw = common::sample_raw(w, h, 16);
        let profile = common::sample_profile();
        let mut dng = Vec::new();
        DngEncoder::new()
            .with_byte_order(order)
            .encode_cfa(&raw, &profile, &mut dng)
            .expect("encode");

        let dec = libtiff_oracle::decode_tiff(&dng).expect("libtiff must parse the DNG's IFD 0");
        // IFD 0 is the RGB preview, one pixel per CFA repeat tile (width/2 x height/2).
        assert_eq!(dec.samples_per_pixel, 3, "preview is RGB");
        assert_eq!((dec.width, dec.height), (w / 2, h / 2));
        assert_eq!(dec.pixels.len() as u32, (w / 2) * (h / 2) * 3);
    }
}
