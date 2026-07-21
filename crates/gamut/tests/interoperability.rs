//! Cross-crate interoperability checks owned by the umbrella layer.

use gamut::core::{Dimensions, EncodeImage, Gray8, ImageRef};

#[test]
fn metadata_facade_round_trips_through_a_jpeg_stream() {
    use gamut::jpeg::{JpegEncoder, metadata};
    use gamut::metadata::exif::{ByteOrder, Exif, ExifTag, Value};
    use gamut::metadata::icc::{ColorSpace, DeviceClass, IccProfile, ProfileHeader};
    use gamut::metadata::xmp::{WellKnownNs, XmpMeta};
    use gamut::metadata::{Metadata, MetadataBlock};

    let mut exif = Exif::new(ByteOrder::LittleEndian);
    exif.set_tag(ExifTag::Make, Value::Ascii("gamut".to_owned()));
    let mut xmp = XmpMeta::new();
    xmp.set_text(WellKnownNs::Xmp.uri(), "CreatorTool", "gamut");
    let icc = IccProfile {
        header: ProfileHeader::new(DeviceClass::Display, ColorSpace::Rgb),
        tags: Vec::new(),
    };
    let typed = Metadata {
        exif: Some(exif),
        xmp: Some(xmp),
        icc: Some(icc),
    };

    let encoded = typed.encode().unwrap();
    let pixels = vec![128u8; 64];
    let image = ImageRef::<Gray8>::new(&pixels, Dimensions::new(8, 8).unwrap()).unwrap();
    let jpeg = JpegEncoder::new()
        .with_exif(encoded.exif.as_deref().unwrap())
        .with_xmp(encoded.xmp.as_deref().unwrap())
        .with_icc_profile(encoded.icc.as_deref().unwrap())
        .encode_to_vec(image)
        .unwrap();

    let read = metadata(&jpeg).unwrap();
    let through_jpeg = Metadata::from_blocks(&[
        MetadataBlock::Exif(read.exif.as_deref().unwrap()),
        MetadataBlock::Xmp(read.xmp.as_deref().unwrap()),
        MetadataBlock::Icc(read.icc.as_deref().unwrap()),
    ])
    .unwrap();
    let direct = Metadata::from_blocks(&[
        MetadataBlock::Exif(encoded.exif.as_deref().unwrap()),
        MetadataBlock::Xmp(encoded.xmp.as_deref().unwrap()),
        MetadataBlock::Icc(encoded.icc.as_deref().unwrap()),
    ])
    .unwrap();

    assert_eq!(through_jpeg, direct);
    assert_eq!(
        through_jpeg.exif.as_ref().and_then(|value| value.make()),
        Some("gamut")
    );
}

#[test]
fn reinhard_matches_the_color_pq_to_sdr_step() {
    use gamut::color::transfer::{bt2020_pq_to_sdr, pq_eotf};
    use gamut::core::luminance::HDR_REFERENCE_WHITE_NITS;
    use gamut::tonemap::{Reinhard, ToneCurve};

    for &signal in &[0.1_f64, 0.25, 0.5, 0.75, 1.0] {
        let linear = pq_eotf(signal) / HDR_REFERENCE_WHITE_NITS;
        let tone_mapped = Reinhard.map(linear as f32);
        let converted = bt2020_pq_to_sdr(signal) as f32;
        assert!(
            (tone_mapped - converted).abs() <= 1e-6,
            "signal {signal}: {tone_mapped} vs {converted}"
        );
    }
}
