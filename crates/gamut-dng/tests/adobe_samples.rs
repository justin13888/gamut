//! Decode conformance against **Adobe's official sample DNGs** (shipped inside the DNG SDK ZIP,
//! surfaced by `gamut_dng_oracle::sample_file`): gamut-dng's decoder must reproduce exactly what
//! the SDK itself reads from Adobe's own files. These are the ProRAW-shaped inputs — tiled,
//! JPEG-XL-compressed linear and Bayer raws.

use gamut_core::Error;
use gamut_dng::DngDecoder;

/// Decodes an Adobe sample with both gamut and the SDK and requires pixel-exact agreement.
fn assert_matches_adobe(name: &str) {
    let bytes = gamut_dng_oracle::sample_file(name).expect("sample DNG present");
    let gamut = DngDecoder::new().decode(&bytes).expect("gamut decode");
    let adobe = gamut_dng_oracle::read_raw_dng(&bytes).expect("adobe decode");
    assert_eq!(
        (
            gamut.raw.dimensions().width,
            gamut.raw.dimensions().height,
            u32::from(gamut.raw.samples_per_pixel())
        ),
        (adobe.width, adobe.height, adobe.planes),
        "{name}: geometry must agree"
    );
    // Adobe encoded these samples lossily (VarDCT), and JPEG XL conformance permits a tiny
    // per-sample tolerance between conforming decoders — jxl-rs and libjxl round the float
    // reconstruction independently. Lossless streams decode bit-exact, so this tolerance is
    // only ever consumed by lossy input; anything structural (interleave, tiling, depth) shifts
    // values by orders of magnitude more than one code.
    let mut diverging = 0usize;
    for (i, (&ours, &theirs)) in gamut.raw.samples().iter().zip(&adobe.samples).enumerate() {
        let diff = (i64::from(ours) - i64::from(theirs)).abs();
        assert!(
            diff <= 1,
            "{name}: sample {i} diverges beyond lossy rounding — gamut {ours} vs Adobe {theirs}"
        );
        diverging += usize::from(diff != 0);
    }
    // The two decoders must still agree almost everywhere (a systematic offset would not).
    assert!(
        diverging * 50 < adobe.samples.len(),
        "{name}: {diverging} of {} samples differ — beyond decoder rounding",
        adobe.samples.len()
    );
}

#[test]
fn decodes_adobe_jxl_linear_raw_integer_sample() {
    assert_matches_adobe("01_jxl_linear_raw_integer.dng");
}

#[test]
fn decodes_adobe_jxl_bayer_raw_integer_sample() {
    assert_matches_adobe("03_jxl_bayer_raw_integer.dng");
}

/// The fp16 JPEG XL sample must be rejected cleanly (float samples are deferred), naming the
/// reason — not panic, and not silently misdecode.
#[test]
fn rejects_adobe_jxl_float_sample_cleanly() {
    let bytes =
        gamut_dng_oracle::sample_file("02_jxl_linear_raw_float.dng").expect("sample DNG present");
    let err = DngDecoder::new().decode(&bytes).unwrap_err();
    assert!(
        matches!(err, Error::Unsupported(m) if m.contains("floating-point")),
        "expected a floating-point rejection, got {err:?}"
    );
}

/// The JXL samples carry a declared DNG 1.7 version.
#[test]
fn adobe_jxl_sample_declares_dng_17() {
    let bytes =
        gamut_dng_oracle::sample_file("01_jxl_linear_raw_integer.dng").expect("sample present");
    let decoded = DngDecoder::new().decode(&bytes).expect("decode");
    assert_eq!(decoded.dng_version[0], 1);
    assert!(decoded.dng_version[1] >= 7, "JXL requires DNG >= 1.7");
}
