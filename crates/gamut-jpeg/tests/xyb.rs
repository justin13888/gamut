//! The XYB colour mode (issue #334): stream structure, error guards, and differential
//! conformance through libjpeg-turbo — decode the passthrough samples, invert the XYB pipeline in
//! the test, and compare against the source.

use gamut_color::transfer::srgb_oetf;
use gamut_color::xyb::{unscale_xyb, xyb_to_linear_srgb};
use gamut_core::{
    DecodeImage, Dimensions, EncodeImage, ErrorKind, Gray8, ImageBuf, ImageRef, Rgb8,
};
use gamut_jpeg::{JpegColorMode, JpegEncoder, XYB_ICC_PROFILE, metadata};

/// A smooth RGB gradient (JPEG-friendly content, so tolerances reflect quantization).
fn rgb_gradient(w: u32, h: u32) -> Vec<u8> {
    let mut px = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            px.push((x * 255 / w.max(1)) as u8);
            px.push((y * 255 / h.max(1)) as u8);
            px.push((((x + y) * 255) / (w + h).max(1)) as u8);
        }
    }
    px
}

/// Inverts the XYB sample pipeline the way an ICC-aware consumer would: scaled-XYB bytes → XYB →
/// linear sRGB → sRGB bytes.
fn xyb_samples_to_srgb(samples: &[u8]) -> Vec<u8> {
    samples
        .as_chunks::<3>()
        .0
        .iter()
        .flat_map(|px| {
            let scaled = [
                f64::from(px[0]) / 255.0,
                f64::from(px[1]) / 255.0,
                f64::from(px[2]) / 255.0,
            ];
            let linear = xyb_to_linear_srgb(unscale_xyb(scaled));
            linear.map(|v| (srgb_oetf(v.clamp(0.0, 1.0)) * 255.0).round() as u8)
        })
        .collect()
}

/// Max absolute per-sample difference and PSNR between equal-length buffers.
fn stats(a: &[u8], b: &[u8]) -> (u8, f64) {
    assert_eq!(a.len(), b.len());
    let mut max = 0u8;
    let mut sse = 0f64;
    for (&x, &y) in a.iter().zip(b) {
        max = max.max(x.abs_diff(y));
        sse += f64::from(x.abs_diff(y)).powi(2);
    }
    let mse = sse / a.len().max(1) as f64;
    let psnr = if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0 * 255.0 / mse).log10()
    };
    (max, psnr)
}

#[test]
fn xyb_stream_structure_is_the_jpegli_convention() {
    let (w, h) = (24u32, 17u32);
    let src = rgb_gradient(w, h);
    let img = ImageRef::<Rgb8>::new(&src, Dimensions::new(w, h).unwrap()).unwrap();
    let jpeg = JpegEncoder::new()
        .with_color_mode(JpegColorMode::Xyb)
        .encode_to_vec(img)
        .unwrap();

    // Walk the header segments: no APP0, APP14 with transform 0 first, exactly one APP2 carrying
    // the XYB profile, a two-table DQT, SOF ids 'R','G','B' all 1×1 with Tq 0,0,1.
    let mut pos = 2; // past SOI
    let mut saw = Vec::new();
    let mut sof = Vec::new();
    let mut app14 = Vec::new();
    while jpeg[pos + 1] != 0xDA {
        let code = jpeg[pos + 1];
        let len = usize::from(u16::from_be_bytes([jpeg[pos + 2], jpeg[pos + 3]]));
        let payload = &jpeg[pos + 4..pos + 2 + len];
        saw.push(code);
        if code == 0xEE {
            app14 = payload.to_vec();
        }
        if code == 0xC0 {
            sof = payload.to_vec();
        }
        pos += 2 + len;
    }
    assert!(
        !saw.contains(&0xE0),
        "an XYB stream must not carry JFIF APP0"
    );
    assert_eq!(saw[0], 0xEE, "APP14 leads the header");
    assert_eq!(&app14[..5], b"Adobe");
    assert_eq!(*app14.last().unwrap(), 0, "transform = 0");
    assert_eq!(saw.iter().filter(|&&c| c == 0xE2).count(), 1, "one APP2");
    // SOF payload: P, Y(2), X(2), Nf, then Nf × (Ci, Hi|Vi, Tqi).
    assert_eq!(sof[5], 3, "three components");
    let comps: Vec<(u8, u8, u8)> = (0..3)
        .map(|i| (sof[6 + i * 3], sof[7 + i * 3], sof[8 + i * 3]))
        .collect();
    assert_eq!(
        comps,
        vec![(b'R', 0x11, 0), (b'G', 0x11, 0), (b'B', 0x11, 1)],
        "ids R,G,B; 1x1 sampling; Tq 0,0,1"
    );

    // The embedded profile is byte-for-byte the vendored one, via the crate's own metadata reader.
    let meta = metadata(&jpeg).unwrap();
    assert_eq!(meta.icc.as_deref(), Some(XYB_ICC_PROFILE));
}

#[test]
fn xyb_byte_count_is_relative_to_appended_output() {
    // The XYB path returns early with its own byte count: encode into a Vec that already holds a
    // 5-byte prefix and check the count covers only the appended stream, prefix left intact.
    let prefix = [0xA1u8, 0xA2, 0xA3, 0xA4, 0xA5];
    let src = rgb_gradient(8, 8);
    let img = ImageRef::<Rgb8>::new(&src, Dimensions::new(8, 8).unwrap()).unwrap();

    let mut out = prefix.to_vec();
    let n = JpegEncoder::new()
        .with_color_mode(JpegColorMode::Xyb)
        .encode_image(img, &mut out)
        .unwrap();

    assert_eq!(n, out.len() - prefix.len());
    assert_eq!(&out[..prefix.len()], &prefix);
    assert_eq!(&out[prefix.len()..prefix.len() + 2], &[0xFF, 0xD8]); // SOI right after the prefix
}

#[test]
fn xyb_round_trips_through_libjpeg_and_through_gamut() {
    // Both decoders see passthrough samples; inverting the XYB pipeline in the test must recover
    // the source within JPEG-lossy tolerance, baseline and progressive, plus a restart cell.
    for &(w, h) in &[(16u32, 16u32), (33, 31), (64, 48)] {
        for (progressive, restart) in [(false, 0u16), (true, 0), (false, 2)] {
            let src = rgb_gradient(w, h);
            let img = ImageRef::<Rgb8>::new(&src, Dimensions::new(w, h).unwrap()).unwrap();
            let jpeg = JpegEncoder::new()
                .with_color_mode(JpegColorMode::Xyb)
                .with_quality(90)
                .with_progressive(progressive)
                .with_restart_interval(restart)
                .encode_to_vec(img)
                .unwrap();

            // libjpeg-turbo: geometry + passthrough decode (no YCbCr inverse thanks to APP14=0).
            let reference = libjpeg_oracle::decode(&jpeg).expect("libjpeg-turbo decode");
            assert_eq!(
                (reference.width, reference.height, reference.channels),
                (w, h, 3)
            );
            let (max, psnr) = stats(&xyb_samples_to_srgb(&reference.pixels), &src);
            // Measured worst over the battery: max-diff 24 (the inherent near-black X
            // amplification the ICC pin test documents), PSNR 40.5 dB. Asserted with margin.
            assert!(
                psnr > 35.0,
                "{w}x{h} prog={progressive} r={restart}: libjpeg path PSNR {psnr:.2} (max {max})"
            );

            // The oracle also reads back the embedded profile.
            assert_eq!(
                libjpeg_oracle::read_icc_profile(&jpeg).expect("read icc"),
                Some(XYB_ICC_PROFILE.to_vec())
            );

            // gamut's own decoder: same passthrough presentation, parity with the oracle within
            // IDCT rounding, and the same reconstruction quality.
            let own: ImageBuf<Rgb8> = gamut_jpeg::JpegDecoder::new().decode_image(&jpeg).unwrap();
            let (parity, _) = stats(own.as_samples(), &reference.pixels);
            assert!(parity <= 3, "gamut vs libjpeg parity {parity}");
            let (_, own_psnr) = stats(&xyb_samples_to_srgb(own.as_samples()), &src);
            assert!(own_psnr > 35.0, "gamut path PSNR {own_psnr:.2}");
        }
    }
}

#[test]
fn xyb_mode_guards_reject_what_it_cannot_describe() {
    let dims = Dimensions::new(8, 8).unwrap();
    // Grayscale input has no XYB representation.
    let gray = ImageRef::<Gray8>::new(&[128u8; 64], dims).unwrap();
    let err = JpegEncoder::new()
        .with_color_mode(JpegColorMode::Xyb)
        .encode_to_vec(gray)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unsupported, "{err:?}");

    // A caller ICC profile would misdescribe XYB samples.
    let rgb = ImageRef::<Rgb8>::new(&[128u8; 8 * 8 * 3], dims).unwrap();
    let err = JpegEncoder::new()
        .with_color_mode(JpegColorMode::Xyb)
        .with_icc_profile(&[0u8; 132])
        .encode_to_vec(rgb)
        .unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidInput, "{err:?}");

    // EXIF still embeds (metadata that describes provenance, not colour).
    let rgb = ImageRef::<Rgb8>::new(&[128u8; 8 * 8 * 3], dims).unwrap();
    let jpeg = JpegEncoder::new()
        .with_color_mode(JpegColorMode::Xyb)
        .with_exif(b"II*\x00\x08\x00\x00\x00\x00\x00")
        .encode_to_vec(rgb)
        .unwrap();
    let meta = metadata(&jpeg).unwrap();
    assert!(meta.exif.is_some());
    assert_eq!(meta.icc.as_deref(), Some(XYB_ICC_PROFILE));
}
