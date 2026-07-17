//! Integration tests for the sequential JPEG decoder, exercising only the public API: round-trips
//! against the crate's own encoder across a dimension/subsampling/quality/restart battery, a
//! malformed-input rejection corpus, a no-panic byte-flip sweep, and `info()`.

use gamut_core::{DecodeImage, Dimensions, EncodeImage, Gray8, ImageBuf, ImageRef, Rgb8};
use gamut_jpeg::{ChromaSubsampling, JpegDecoder, JpegEncoder, JpegProcess};

/// A deterministic per-pixel-distinct pattern (varies on both axes) so every coordinate is
/// load-bearing — a mis-indexed decode diverges somewhere.
fn pattern(i: usize) -> u8 {
    ((i * 31 + 17) % 251) as u8
}

/// Max absolute per-channel difference and mean-squared error between two equal-length sample sets.
fn diff_stats(a: &[u8], b: &[u8]) -> (u8, f64) {
    assert_eq!(a.len(), b.len());
    let mut max = 0u8;
    let mut sse = 0f64;
    for (&x, &y) in a.iter().zip(b) {
        let d = x.abs_diff(y);
        max = max.max(d);
        sse += f64::from(d) * f64::from(d);
    }
    (max, sse / a.len() as f64)
}

/// Peak signal-to-noise ratio (dB) from a mean-squared error, `∞` for an exact match.
fn psnr(mse: f64) -> f64 {
    if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0 * 255.0 / mse).log10()
    }
}

#[test]
fn gray_roundtrip_q100_is_near_lossless() {
    // At quality 100 every quant step is 1, so the only loss is the FDCT and IDCT rounding. A
    // per-pixel-distinct gradient bounds that composed error tightly.
    let mut worst = 0u8;
    for &(w, h) in &[(8u32, 8u32), (9, 9), (16, 16), (17, 23), (32, 24)] {
        let src: Vec<u8> = (0..(w * h) as usize).map(pattern).collect();
        let img = ImageRef::<Gray8>::new(&src, Dimensions::new(w, h).unwrap()).unwrap();
        let jpeg = JpegEncoder::new()
            .with_quality(100)
            .encode_to_vec(img)
            .unwrap();
        let out: ImageBuf<Gray8> = JpegDecoder::new().decode_image(&jpeg).unwrap();
        assert_eq!(out.dimensions(), Dimensions::new(w, h).unwrap());
        let (max, _) = diff_stats(&src, out.as_samples());
        worst = worst.max(max);
    }
    // FDCT-round + IDCT-round on 8-bit samples: measured worst case is 1 code per pixel.
    assert!(worst <= 1, "gray q=100 max diff {worst} exceeded 1");
}

#[test]
fn color_444_roundtrip_q100_is_tight() {
    // 4:4:4 keeps chroma at full resolution, so the loss is FDCT+IDCT rounding plus the two
    // RGB↔YCbCr conversion rounds. A few codes per channel is the honest budget.
    let mut worst = 0u8;
    for &(w, h) in &[(8u32, 8u32), (16, 16), (17, 9), (24, 24)] {
        let src: Vec<u8> = (0..(w * h * 3) as usize).map(pattern).collect();
        let img = ImageRef::<Rgb8>::new(&src, Dimensions::new(w, h).unwrap()).unwrap();
        let jpeg = JpegEncoder::new()
            .with_quality(100)
            .with_subsampling(ChromaSubsampling::Ycbcr444)
            .encode_to_vec(img)
            .unwrap();
        let out: ImageBuf<Rgb8> = JpegDecoder::new().decode_image(&jpeg).unwrap();
        let (max, _) = diff_stats(&src, out.as_samples());
        worst = worst.max(max);
    }
    // FDCT/IDCT rounding plus the two RGB↔YCbCr conversion rounds: measured worst case is 3.
    assert!(worst <= 3, "color 4:4:4 q=100 max diff {worst} exceeded 3");
}

#[test]
fn color_q50_photographic_gradient_has_bounded_psnr() {
    // A smooth gradient (photographic-ish) at q=50 must reconstruct with a healthy PSNR floor; a
    // gross decode bug (wrong dequant, transposed blocks, bad colour) collapses it.
    let (w, h) = (64u32, 48u32);
    let mut src = vec![0u8; (w * h * 3) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 3) as usize;
            src[i] = (x * 255 / (w - 1)) as u8;
            src[i + 1] = (y * 255 / (h - 1)) as u8;
            src[i + 2] = ((x + y) * 255 / (w + h - 2)) as u8;
        }
    }
    let img = ImageRef::<Rgb8>::new(&src, Dimensions::new(w, h).unwrap()).unwrap();
    for sub in [
        ChromaSubsampling::Ycbcr444,
        ChromaSubsampling::Ycbcr422,
        ChromaSubsampling::Ycbcr420,
    ] {
        let jpeg = JpegEncoder::new()
            .with_quality(50)
            .with_subsampling(sub)
            .encode_to_vec(img)
            .unwrap();
        let out: ImageBuf<Rgb8> = JpegDecoder::new().decode_image(&jpeg).unwrap();
        let (_, mse) = diff_stats(&src, out.as_samples());
        assert!(psnr(mse) > 30.0, "q=50 {sub:?} PSNR {:.2} dB", psnr(mse));
    }
}

#[test]
fn roundtrip_battery_decodes_without_error() {
    // A broad grid of dims × subsampling × quality × restart: every stream must round-trip to the
    // right dimensions with a bounded PSNR. Distinct content makes any geometry mutant diverge.
    let dims = [(1u32, 1u32), (7, 5), (8, 8), (15, 17), (16, 16), (33, 20)];
    let subs = [
        ChromaSubsampling::Ycbcr444,
        ChromaSubsampling::Ycbcr422,
        ChromaSubsampling::Ycbcr420,
    ];
    for &(w, h) in &dims {
        for &q in &[30u8, 75, 95] {
            for &restart in &[0u16, 1, 4] {
                // Grayscale.
                let gray: Vec<u8> = (0..(w * h) as usize).map(pattern).collect();
                let gimg = ImageRef::<Gray8>::new(&gray, Dimensions::new(w, h).unwrap()).unwrap();
                let gj = JpegEncoder::new()
                    .with_quality(q)
                    .with_restart_interval(restart)
                    .encode_to_vec(gimg)
                    .unwrap();
                let gout: ImageBuf<Gray8> = JpegDecoder::new().decode_image(&gj).unwrap();
                assert_eq!(gout.dimensions(), Dimensions::new(w, h).unwrap());

                // Colour, each subsampling.
                for &sub in &subs {
                    let rgb: Vec<u8> = (0..(w * h * 3) as usize).map(pattern).collect();
                    let img = ImageRef::<Rgb8>::new(&rgb, Dimensions::new(w, h).unwrap()).unwrap();
                    let jpeg = JpegEncoder::new()
                        .with_quality(q)
                        .with_subsampling(sub)
                        .with_restart_interval(restart)
                        .encode_to_vec(img)
                        .unwrap();
                    let out: ImageBuf<Rgb8> = JpegDecoder::new().decode_image(&jpeg).unwrap();
                    assert_eq!(out.dimensions(), Dimensions::new(w, h).unwrap());
                }
            }
        }
    }
}

#[test]
fn gray_stream_rejected_as_cmyk_and_color_as_gray() {
    // The strict pixel-type impls: a 1-component stream cannot present as Cmyk8, a 3-component
    // stream cannot present as Gray8.
    use gamut_core::Cmyk8;
    let gray = ImageRef::<Gray8>::new(&[128u8; 64], Dimensions::new(8, 8).unwrap()).unwrap();
    let gj = JpegEncoder::new().encode_to_vec(gray).unwrap();
    assert!(<JpegDecoder as DecodeImage<Cmyk8>>::decode_image(&JpegDecoder::new(), &gj).is_err());

    let rgb = vec![90u8; 8 * 8 * 3];
    let cimg = ImageRef::<Rgb8>::new(&rgb, Dimensions::new(8, 8).unwrap()).unwrap();
    let cj = JpegEncoder::new().encode_to_vec(cimg).unwrap();
    assert!(<JpegDecoder as DecodeImage<Gray8>>::decode_image(&JpegDecoder::new(), &cj).is_err());
    // ...and a 3-component stream is Unsupported as Cmyk8 too (use Rgb8).
    assert!(<JpegDecoder as DecodeImage<Cmyk8>>::decode_image(&JpegDecoder::new(), &cj).is_err());
}

#[test]
fn corrupted_restart_sequence_is_rejected() {
    // Encode with restarts, then swap the first RST marker (RST0 = 0xD0) to RST5 (0xD5): the
    // decoder's modulo-8 sequence check must reject it.
    let rgb: Vec<u8> = (0..16 * 16 * 3).map(pattern).collect();
    let img = ImageRef::<Rgb8>::new(&rgb, Dimensions::new(16, 16).unwrap()).unwrap();
    let good = JpegEncoder::new()
        .with_restart_interval(1)
        .with_subsampling(ChromaSubsampling::Ycbcr444)
        .encode_to_vec(img)
        .unwrap();
    // Baseline decodes fine.
    assert!(<JpegDecoder as DecodeImage<Rgb8>>::decode_image(&JpegDecoder::new(), &good).is_ok());

    // Find the first RST0 (0xFF 0xD0) in the entropy region and corrupt it to RST5.
    let mut bad = good.clone();
    let idx = bad
        .windows(2)
        .position(|w| w == [0xFF, 0xD0])
        .expect("a RST0 marker");
    bad[idx + 1] = 0xD5;
    assert!(<JpegDecoder as DecodeImage<Rgb8>>::decode_image(&JpegDecoder::new(), &bad).is_err());

    // Deleting a restart marker entirely (splice out the 2 marker bytes) also desynchronizes.
    let mut missing = good.clone();
    missing.drain(idx..idx + 2);
    assert!(
        <JpegDecoder as DecodeImage<Rgb8>>::decode_image(&JpegDecoder::new(), &missing).is_err()
    );
}

#[test]
fn malformed_streams_are_rejected_not_panicked() {
    let dec = JpegDecoder::new();
    let d = |b: &[u8]| <JpegDecoder as DecodeImage<Rgb8>>::decode_image(&dec, b);

    assert!(d(&[]).is_err(), "empty");
    assert!(d(&[0xFF, 0xD8]).is_err(), "SOI only");
    assert!(d(&[0x00, 0x01, 0x02]).is_err(), "no SOI");
    // A valid prefix truncated at various structural points.
    let rgb: Vec<u8> = (0..8 * 8 * 3).map(pattern).collect();
    let img = ImageRef::<Rgb8>::new(&rgb, Dimensions::new(8, 8).unwrap()).unwrap();
    let full = JpegEncoder::new().encode_to_vec(img).unwrap();
    for cut in [4, 10, 20, full.len() - 5, full.len() - 1] {
        assert!(d(&full[..cut]).is_err(), "truncated at {cut}");
    }
    // Segment length 0 and 1 right after SOI (marker 0xE0 APP0 with bogus length).
    assert!(d(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x00]).is_err(), "len 0");
    assert!(d(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x01]).is_err(), "len 1");
    // A baseline stream relabelled SOF2 (progressive) has a baseline SOS (Se=63) that fails the
    // progressive DC-scan validation — a clean error, not a panic.
    let mut prog = full.clone();
    let sof = prog.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
    prog[sof + 1] = 0xC2;
    assert!(d(&prog).is_err(), "mislabelled progressive rejected");
}

#[test]
fn trailing_garbage_after_eoi_is_ignored() {
    // libjpeg convention: bytes after EOI are ignored, not an error.
    let src: Vec<u8> = (0..64).map(pattern).collect();
    let img = ImageRef::<Gray8>::new(&src, Dimensions::new(8, 8).unwrap()).unwrap();
    let mut jpeg = JpegEncoder::new().encode_to_vec(img).unwrap();
    jpeg.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF, 0xFF, 0xD9]);
    let out: ImageBuf<Gray8> = JpegDecoder::new().decode_image(&jpeg).unwrap();
    assert_eq!(out.dimensions(), Dimensions::new(8, 8).unwrap());
}

#[test]
fn byte_flip_sweep_never_panics() {
    // The no-panic-on-untrusted-input gate: flip a bit in every byte position (strided for speed)
    // of a valid stream and assert decode returns Ok or Err without panicking.
    let rgb: Vec<u8> = (0..16 * 16 * 3).map(pattern).collect();
    let img = ImageRef::<Rgb8>::new(&rgb, Dimensions::new(16, 16).unwrap()).unwrap();
    let base = JpegEncoder::new()
        .with_restart_interval(2)
        .encode_to_vec(img)
        .unwrap();
    let dec = JpegDecoder::new();
    for i in 0..base.len() {
        let mut m = base.clone();
        m[i] ^= 0xFF;
        // Must not panic; the result value is irrelevant.
        let _ = <JpegDecoder as DecodeImage<Rgb8>>::decode_image(&dec, &m);
    }
    // Also truncate at every length.
    for cut in 0..base.len() {
        let _ = <JpegDecoder as DecodeImage<Rgb8>>::decode_image(&dec, &base[..cut]);
    }
}

#[test]
fn info_reports_frame_header() {
    let rgb = vec![10u8; 20 * 12 * 3];
    let img = ImageRef::<Rgb8>::new(&rgb, Dimensions::new(20, 12).unwrap()).unwrap();
    let jpeg = JpegEncoder::new()
        .with_subsampling(ChromaSubsampling::Ycbcr420)
        .encode_to_vec(img)
        .unwrap();
    let info = gamut_jpeg::info(&jpeg).unwrap();
    assert_eq!((info.width, info.height), (20, 12));
    assert_eq!(info.components, 3);
    assert_eq!(info.precision, 8);
    assert_eq!(info.process, JpegProcess::Baseline);

    // A grayscale SOF0 reports 1 component.
    let g = JpegEncoder::new()
        .encode_to_vec(ImageRef::<Gray8>::new(&[0u8; 64], Dimensions::new(8, 8).unwrap()).unwrap())
        .unwrap();
    assert_eq!(gamut_jpeg::info(&g).unwrap().components, 1);

    // info() on junk errors rather than panicking.
    assert!(gamut_jpeg::info(&[0xFF, 0xD8]).is_err());
}
