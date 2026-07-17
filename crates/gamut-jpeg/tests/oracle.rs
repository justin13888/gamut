//! Differential conformance cross-checks against a vendored **libjpeg-turbo** (v3.x).
//!
//! Two directions prove the codec against the canonical reference implementation:
//!
//! - **Encode** — gamut encodes, libjpeg-turbo decodes; the recovered pixels must match the source
//!   within the format's lossy tolerance. This is the real point of the gate: it proves gamut emits
//!   spec-valid streams the reference decoder reads back correctly.
//! - **Decode** — libjpeg-turbo encodes, gamut decodes; gamut's output must match libjpeg-turbo's
//!   own decode of the *same* stream. Decode-vs-decode parity isolates entropy/dequant/IDCT
//!   correctness from the lossy encode error (both decoders see identical coefficients).
//!
//! Tolerances are **measured**, then asserted with a margin; each bound cites the measured worst
//! case in a comment. Content is deterministic (a smooth gradient for fidelity-vs-source, a fixed
//! LCG for the geometry-sensitive parity checks) so the numbers are stable across runs.

use gamut_core::{DecodeImage, Dimensions, EncodeImage, Gray8, ImageBuf, ImageRef, Rgb8};
use gamut_jpeg::{ChromaSubsampling, JpegDecoder, JpegEncoder};
use libjpeg_oracle::{EncodeParams, Subsampling};

/// The dimension battery: block-aligned, odd, and larger-than-MCU cases so component-dimension
/// arithmetic (§A.1.1) and edge padding are all exercised.
const DIMS: &[(u32, u32)] = &[(8, 8), (16, 16), (17, 9), (33, 31), (64, 48)];

/// One colour configuration, shared by both directions.
#[derive(Clone, Copy, Debug)]
enum Mode {
    Gray,
    C444,
    C422,
    C420,
}

impl Mode {
    const ALL: [Mode; 4] = [Mode::Gray, Mode::C444, Mode::C422, Mode::C420];

    fn channels(self) -> u32 {
        match self {
            Mode::Gray => 1,
            _ => 3,
        }
    }

    fn gamut_subsampling(self) -> ChromaSubsampling {
        match self {
            Mode::Gray | Mode::C444 => ChromaSubsampling::Ycbcr444,
            Mode::C422 => ChromaSubsampling::Ycbcr422,
            Mode::C420 => ChromaSubsampling::Ycbcr420,
        }
    }

    fn oracle_subsampling(self) -> Subsampling {
        match self {
            Mode::C422 => Subsampling::S422,
            Mode::C420 => Subsampling::S420,
            _ => Subsampling::S444,
        }
    }

    /// Chroma is upsampled by different filters (gamut = sample replication, libjpeg = fancy
    /// triangle), so subsampled modes diverge legitimately; only gray and 4:4:4 avoid that.
    fn upsampling_matches(self) -> bool {
        matches!(self, Mode::Gray | Mode::C444)
    }
}

// --- content generators -------------------------------------------------------------------------

/// A smooth RGB gradient — JPEG-friendly (low high-frequency energy), so fidelity-vs-source
/// tolerances reflect quantization rather than ringing.
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

/// The single-channel luma companion of [`rgb_gradient`].
fn gray_gradient(w: u32, h: u32) -> Vec<u8> {
    let mut px = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        for x in 0..w {
            px.push((((x * 255 / w.max(1)) as u16 + (y * 255 / h.max(1)) as u16) / 2) as u8);
        }
    }
    px
}

/// A fixed LCG (Numerical Recipes constants) producing deterministic, high-frequency content — the
/// worst case for IDCT-rounding parity and for the upsampling-filter divergence.
struct Lcg(u32);

impl Lcg {
    fn new(seed: u32) -> Self {
        Self(seed)
    }
    fn byte(&mut self) -> u8 {
        self.0 = self.0.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.0 >> 24) as u8
    }
}

/// `w * h * channels` deterministic LCG bytes.
fn lcg_pixels(w: u32, h: u32, channels: u32, seed: u32) -> Vec<u8> {
    let mut lcg = Lcg::new(seed);
    (0..(w * h * channels) as usize)
        .map(|_| lcg.byte())
        .collect()
}

// --- metrics ------------------------------------------------------------------------------------

/// Max absolute per-sample difference and the mean-squared error between equal-length buffers.
fn diff_stats(a: &[u8], b: &[u8]) -> (u8, f64) {
    assert_eq!(a.len(), b.len(), "compared buffers differ in length");
    let mut max = 0u8;
    let mut sse = 0f64;
    for (&x, &y) in a.iter().zip(b) {
        let d = x.abs_diff(y);
        max = max.max(d);
        sse += f64::from(d) * f64::from(d);
    }
    (max, sse / a.len().max(1) as f64)
}

/// Peak signal-to-noise ratio (dB); `∞` for an exact match.
fn psnr(mse: f64) -> f64 {
    if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0 * 255.0 / mse).log10()
    }
}

// --- gamut helpers ------------------------------------------------------------------------------

/// Encodes `pixels` with gamut in the given mode, returning the JPEG stream.
fn gamut_encode(mode: Mode, pixels: &[u8], w: u32, h: u32, quality: u8, restart: u16) -> Vec<u8> {
    let dims = Dimensions::new(w, h).unwrap();
    let enc = JpegEncoder::new()
        .with_quality(quality)
        .with_restart_interval(restart);
    match mode {
        Mode::Gray => enc
            .encode_to_vec(ImageRef::<Gray8>::new(pixels, dims).unwrap())
            .unwrap(),
        _ => enc
            .with_subsampling(mode.gamut_subsampling())
            .encode_to_vec(ImageRef::<Rgb8>::new(pixels, dims).unwrap())
            .unwrap(),
    }
}

/// Decodes `jpeg` with gamut into interleaved samples (1 channel for gray, 3 for colour).
fn gamut_decode(mode: Mode, jpeg: &[u8]) -> (u32, u32, Vec<u8>) {
    let dec = JpegDecoder::new();
    match mode {
        Mode::Gray => {
            let img: ImageBuf<Gray8> = dec.decode_image(jpeg).unwrap();
            (img.width(), img.height(), img.as_samples().to_vec())
        }
        _ => {
            let img: ImageBuf<Rgb8> = dec.decode_image(jpeg).unwrap();
            (img.width(), img.height(), img.as_samples().to_vec())
        }
    }
}

/// Encodes `pixels` with the libjpeg-turbo oracle in the given mode.
fn oracle_encode(
    mode: Mode,
    pixels: &[u8],
    w: u32,
    h: u32,
    quality: i32,
    restart: u16,
    optimize: bool,
) -> Vec<u8> {
    let params = EncodeParams {
        quality,
        gray: matches!(mode, Mode::Gray),
        subsampling: mode.oracle_subsampling(),
        progressive: false,
        restart_interval: restart,
        optimize_coding: optimize,
    };
    libjpeg_oracle::encode(pixels, w, h, &params).expect("oracle encode")
}

// ================================================================================================
// Encode direction: gamut encodes → libjpeg-turbo decodes.
// ================================================================================================

#[test]
fn encode_battery_decoded_by_libjpeg_turbo() {
    // dims × mode × quality × restart. libjpeg-turbo must decode every stream with the right
    // geometry (stream validity — the real point), and the pixels must be faithful to the SOURCE.
    let mut worst_gray444 = 0u8; // max per-pixel diff, gray + 4:4:4
    let mut worst_sub_mse = 0f64; // worst MSE, 4:2:2 + 4:2:0
    for &(w, h) in DIMS {
        for mode in Mode::ALL {
            for &q in &[50u8, 75, 90] {
                for &restart in &[0u16, 3] {
                    let src = if matches!(mode, Mode::Gray) {
                        gray_gradient(w, h)
                    } else {
                        rgb_gradient(w, h)
                    };
                    let jpeg = gamut_encode(mode, &src, w, h, q, restart);
                    let img = libjpeg_oracle::decode(&jpeg).expect("libjpeg-turbo decode");
                    assert_eq!(
                        (img.width, img.height, img.channels),
                        (w, h, mode.channels()),
                        "geometry {mode:?} {w}x{h} q{q} r{restart}"
                    );
                    let (max, mse) = diff_stats(&img.pixels, &src);
                    if mode.upsampling_matches() {
                        worst_gray444 = worst_gray444.max(max);
                    } else {
                        worst_sub_mse = worst_sub_mse.max(mse);
                    }
                }
            }
        }
    }
    // Measured worst over the battery (q ∈ {50,75,90}): gray/444 max-diff = 11 (the q50 cell
    // dominates); subsampled PSNR = 30.11 dB. Asserted with margin.
    assert!(worst_gray444 <= 16, "gray/444 max-diff {worst_gray444}");
    assert!(
        psnr(worst_sub_mse) > 28.0,
        "subsampled PSNR {:.2} dB",
        psnr(worst_sub_mse)
    );
}

#[test]
fn encode_q90_is_tight_against_source() {
    // A focused high-quality cell: at q90 the gradient should reconstruct nearly exactly through
    // the reference decoder. Gray/444 assert a small per-pixel cap; subsampled a high PSNR floor.
    let mut worst_gray444 = 0u8;
    let mut worst_sub_mse = 0f64;
    for &(w, h) in DIMS {
        for mode in Mode::ALL {
            let src = if matches!(mode, Mode::Gray) {
                gray_gradient(w, h)
            } else {
                rgb_gradient(w, h)
            };
            let jpeg = gamut_encode(mode, &src, w, h, 90, 0);
            let img = libjpeg_oracle::decode(&jpeg).expect("decode");
            let (max, mse) = diff_stats(&img.pixels, &src);
            if mode.upsampling_matches() {
                worst_gray444 = worst_gray444.max(max);
            } else {
                worst_sub_mse = worst_sub_mse.max(mse);
            }
        }
    }
    // Measured at q90: gray/444 max-diff = 5; subsampled PSNR = 34.18 dB. Asserted with margin.
    assert!(worst_gray444 <= 8, "q90 gray/444 max-diff {worst_gray444}");
    assert!(
        psnr(worst_sub_mse) > 31.0,
        "q90 sub PSNR {:.2} dB",
        psnr(worst_sub_mse)
    );
}

#[test]
fn encode_cross_parity_gamut_vs_libjpeg() {
    // Same gamut stream, decoded by gamut and by libjpeg-turbo. For gray/4:4:4 (no upsampling-filter
    // divergence) the two decodes differ only by IDCT rounding — a small per-pixel delta. For 4:2:0
    // the upsampling filters legitimately differ (gamut = replication, libjpeg = fancy/triangle), so
    // only a PSNR floor is asserted.
    let mut worst_tight = 0u8;
    let mut worst_420_mse = 0f64;
    for &(w, h) in &[(16u32, 16u32), (33, 31), (64, 48)] {
        for mode in [Mode::Gray, Mode::C444, Mode::C420] {
            let src = if matches!(mode, Mode::Gray) {
                gray_gradient(w, h)
            } else {
                rgb_gradient(w, h)
            };
            let jpeg = gamut_encode(mode, &src, w, h, 85, 0);
            let oracle = libjpeg_oracle::decode(&jpeg).expect("oracle decode");
            let (_, _, gamut) = gamut_decode(mode, &jpeg);
            let (max, mse) = diff_stats(&gamut, &oracle.pixels);
            if mode.upsampling_matches() {
                worst_tight = worst_tight.max(max);
            } else {
                worst_420_mse = worst_420_mse.max(mse);
            }
        }
    }
    // Measured (q85): gray/444 decode-vs-decode max-diff = 2 (IDCT rounding only); 4:2:0 PSNR =
    // 35.22 dB (bounded by the replication-vs-fancy upsampling divergence). Asserted with margin.
    assert!(worst_tight <= 4, "gray/444 parity max-diff {worst_tight}");
    assert!(
        psnr(worst_420_mse) > 32.0,
        "4:2:0 parity PSNR {:.2} dB",
        psnr(worst_420_mse)
    );
}

// ================================================================================================
// Decode direction: libjpeg-turbo encodes → gamut decodes.
// ================================================================================================

#[test]
fn decode_matches_libjpeg_turbo_own_decode() {
    // libjpeg-turbo encodes; gamut decodes and must match libjpeg-turbo's OWN decode of the same
    // stream (decode-vs-decode parity — no lossy encode error in the comparison). The `optimize`
    // cell exercises arbitrary (non-standard) DHT tables; the `restart` cell exercises RSTn resync.
    let mut worst_tight = 0u8;
    let mut worst_sub_mse = 0f64;
    for &(w, h) in &[(16u32, 16u32), (17, 9), (33, 31), (64, 48)] {
        for mode in Mode::ALL {
            for &q in &[40i32, 75, 95] {
                for &(restart, optimize) in &[(0u16, false), (2u16, true)] {
                    let src = lcg_pixels(w, h, mode.channels(), 0x1234_5678);
                    let jpeg = oracle_encode(mode, &src, w, h, q, restart, optimize);
                    let oracle = libjpeg_oracle::decode(&jpeg).expect("oracle decode");
                    assert_eq!(
                        (oracle.width, oracle.height, oracle.channels),
                        (w, h, mode.channels())
                    );
                    let (dw, dh, gamut) = gamut_decode(mode, &jpeg);
                    assert_eq!(
                        (dw, dh, gamut.len()),
                        (w, h, oracle.pixels.len()),
                        "geometry {mode:?} {w}x{h} q{q} r{restart} opt{optimize}"
                    );
                    let (max, mse) = diff_stats(&gamut, &oracle.pixels);
                    if mode.upsampling_matches() {
                        worst_tight = worst_tight.max(max);
                    } else {
                        worst_sub_mse = worst_sub_mse.max(mse);
                    }
                }
            }
        }
    }
    // Measured over q ∈ {40,75,95} on high-frequency LCG content: gray/444 decode-parity max-diff =
    // 3 (IDCT rounding only); subsampled PSNR = 22.91 dB (upsampling-filter divergence, worst on
    // random content). Asserted with margin.
    assert!(
        worst_tight <= 6,
        "gray/444 decode-parity max-diff {worst_tight}"
    );
    assert!(
        psnr(worst_sub_mse) > 20.0,
        "subsampled decode-parity PSNR {:.2} dB",
        psnr(worst_sub_mse)
    );
}

#[test]
fn decode_progressive_is_unsupported_for_now() {
    // libjpeg-turbo emits a progressive (SOF2) stream; the sequential decoder must reject it with a
    // clean Unsupported error whose message names the progressive process (never a panic).
    // P4: this flips to a decode-vs-decode parity test once the progressive decoder lands.
    let (w, h) = (32u32, 32u32);
    let src = rgb_gradient(w, h);
    let params = EncodeParams {
        quality: 85,
        progressive: true,
        subsampling: Subsampling::S444,
        ..EncodeParams::default()
    };
    let jpeg = libjpeg_oracle::encode(&src, w, h, &params).expect("oracle progressive encode");
    let err = <JpegDecoder as DecodeImage<Rgb8>>::decode_image(&JpegDecoder::new(), &jpeg)
        .expect_err("progressive must be rejected");
    let msg = err.to_string();
    assert!(
        msg.to_lowercase().contains("progressive"),
        "error should mention progressive, got: {msg}"
    );
}

#[test]
fn oracle_is_libjpeg_turbo_3() {
    // Guards against accidental submodule drift to a different major version.
    let v = libjpeg_oracle::version();
    assert!(v.starts_with("3."), "expected libjpeg-turbo 3.x, got {v:?}");
}
