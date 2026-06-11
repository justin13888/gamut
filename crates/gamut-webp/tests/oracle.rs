//! Differential oracle (libwebp) for the WebP codecs — lossless (VP8L) and lossy (VP8).
//!
//! libwebp is the third-party reference. Lossy is checked at the YUV-plane level (RGB↔YCbCr is
//! implementation-defined and off the bit-exact gate), in both directions:
//!   - **gamut encode → libwebp decode == gamut decode**, across every encoder feature (prediction,
//!     loop filters, segmentation, token partitions, skip) — pinning gamut's streams as conformant;
//!   - **libwebp encode → gamut decode == libwebp decode**, bit-exact — pinning gamut's decoder
//!     against the full feature surface a production encoder emits (per-segment filter levels,
//!     probability updates, …);
//!   - the lossless round-trips (libwebp self-round-trip; gamut↔libwebp) that shipped with VP8L.

mod common;

use common::{
    libwebp_decode_rgba, libwebp_encode_lossless_rgba, libwebp_get_info, pattern_rgba,
    photo_like_rgba,
};
use gamut_webp::{Dimensions, WebpDecoder, WebpEncoder};

/// The standard dimension matrix exercised by the differential tests, including the awkward
/// single-row / single-column / non-power-of-two cases.
const DIMENSIONS: &[(u32, u32)] = &[
    (1, 1),
    (2, 2),
    (16, 16),
    (17, 9),
    (64, 48),
    (255, 1),
    (1, 255),
];

/// Larger canvases that push past the small-block regime the `DIMENSIONS` matrix (≤255px) stays in:
/// VP8 macroblock grids spanning many rows, and VP8L entropy-image regions plus LZ77 back-references
/// whose distances only exceed 256 above this size. `(300, 70)` is deliberately not a multiple of
/// libwebp's histogram block size, so it straddles entropy-image tile boundaries.
const LARGE_DIMENSIONS: &[(u32, u32)] =
    &[(256, 256), (384, 288), (640, 480), (1024, 768), (300, 70)];

/// Drops the alpha byte of an interleaved RGBA buffer, yielding interleaved RGB.
fn rgba_to_rgb(rgba: &[u8]) -> Vec<u8> {
    rgba.chunks_exact(4)
        .flat_map(|p| [p[0], p[1], p[2]])
        .collect()
}

#[test]
fn libwebp_lossless_self_roundtrip() {
    // Encode → get_info → decode entirely within libwebp: the wrappers and the linked libwebp build
    // are correct iff a fully-opaque image survives a lossless round-trip bit-exactly.
    for (w, h) in [(1u32, 1u32), (16, 16), (17, 9), (64, 48)] {
        let rgba = pattern_rgba(w, h);
        let webp = libwebp_encode_lossless_rgba(&rgba, w, h);
        assert!(!webp.is_empty(), "encode produced no bytes at {w}x{h}");
        assert_eq!(
            libwebp_get_info(&webp),
            Some((w, h)),
            "get_info mismatch at {w}x{h}"
        );
        let decoded = libwebp_decode_rgba(&webp);
        assert_eq!((decoded.width, decoded.height), (w, h));
        assert_eq!(
            decoded.rgba, rgba,
            "lossless must round-trip bit-exactly at {w}x{h}"
        );
    }
}

#[test]
fn gamut_decodes_libwebp_lossless_to_source() {
    // libwebp encodes (using whatever transforms/LZ77/cache it likes); gamut must decode back to the
    // exact source pixels. This is the lossless guarantee end to end through the native decoder.
    for &(w, h) in DIMENSIONS {
        let rgba = pattern_rgba(w, h);
        let webp = libwebp_encode_lossless_rgba(&rgba, w, h);
        let mut out = Vec::new();
        let dims = WebpDecoder::new()
            .decode_to_rgb8(&webp, &mut out)
            .expect("gamut decode");
        assert_eq!(
            (dims.width, dims.height),
            (w, h),
            "dims mismatch at {w}x{h}"
        );
        assert_eq!(out, rgba_to_rgb(&rgba), "pixel mismatch at {w}x{h}");
    }
}

#[test]
fn libwebp_decodes_gamut_lossless_to_source() {
    // The reverse direction: gamut encodes, libwebp (the reference) decodes, and must recover the
    // source pixels — proving gamut emits a conformant lossless stream.
    for &(w, h) in DIMENSIONS {
        let rgb = rgba_to_rgb(&pattern_rgba(w, h));
        let mut webp = Vec::new();
        WebpEncoder::lossless()
            .encode_rgb8(
                &rgb,
                Dimensions {
                    width: w,
                    height: h,
                },
                &mut webp,
            )
            .expect("gamut encode");
        assert_eq!(libwebp_get_info(&webp), Some((w, h)), "get_info at {w}x{h}");
        let decoded = libwebp_decode_rgba(&webp);
        assert_eq!((decoded.width, decoded.height), (w, h));
        assert_eq!(rgba_to_rgb(&decoded.rgba), rgb, "pixel mismatch at {w}x{h}");
    }
}

/// Encodes interleaved RGB with gamut and decodes it with libwebp, asserting the pixels survive.
fn assert_gamut_encode_libwebp_decode(rgb: &[u8], w: u32, h: u32, label: &str) {
    let mut webp = Vec::new();
    WebpEncoder::lossless()
        .encode_rgb8(
            rgb,
            Dimensions {
                width: w,
                height: h,
            },
            &mut webp,
        )
        .expect("gamut encode");
    let decoded = libwebp_decode_rgba(&webp);
    assert_eq!((decoded.width, decoded.height), (w, h), "dims for {label}");
    assert_eq!(rgba_to_rgb(&decoded.rgba), rgb, "pixels for {label}");
}

#[test]
fn libwebp_decodes_every_gamut_encoder_path() {
    // Each image steers gamut's encoder down a different path; libwebp must decode them all.
    let (w, h) = (40u32, 40u32);
    let n = (w * h) as usize;

    // Solid color → palette transform with 8-pixel bundling.
    let solid: Vec<u8> = [30u8, 60, 90].repeat(n);
    assert_gamut_encode_libwebp_decode(&solid, w, h, "solid");

    // Few colors, repetitive → palette + color cache + LZ77.
    let palette = [[10u8, 20, 30], [40, 50, 60], [70, 80, 90]];
    let few: Vec<u8> = (0..n).flat_map(|i| palette[i % 3]).collect();
    assert_gamut_encode_libwebp_decode(&few, w, h, "few-color");

    // 32 colors split top/bottom → palette + multi-group entropy image.
    let regioned: Vec<u8> = (0..n)
        .flat_map(|i| {
            let (x, y) = (i as u32 % w, i as u32 / w);
            let scatter = ((x * 7 + y * 11) % 16) as u8;
            let base = if y < h / 2 { 0 } else { 16 };
            let idx = base + scatter;
            [idx, idx.wrapping_mul(7), idx.wrapping_mul(13)]
        })
        .collect();
    assert_gamut_encode_libwebp_decode(&regioned, w, h, "multi-region");

    // Many colors → spatial transforms (subtract-green/predictor/color) + LZ77 + cache.
    let many: Vec<u8> = (0..n)
        .flat_map(|i| {
            let (x, y) = (i as u32 % w, i as u32 / w);
            [
                (x * 9 + y * 5) as u8,
                (x * 13 + y * 7) as u8,
                (x * 17 + y * 3) as u8,
            ]
        })
        .collect();
    assert_gamut_encode_libwebp_decode(&many, w, h, "many-color");
}

#[test]
fn gamut_decodes_libwebp_lossless_realistic_and_large() {
    // Photographic-like content over the small matrix *and* large canvases (>256px): libwebp encodes
    // with whatever transforms/entropy-images/long back-references it likes, and gamut must decode
    // back to the exact source. This reaches VP8L decode paths (multi-tile entropy images, long LZ77
    // distances) that the ≤255px algebraic patterns never exercise.
    for &(w, h) in DIMENSIONS.iter().chain(LARGE_DIMENSIONS) {
        let rgba = photo_like_rgba(w, h, 0x51ed);
        let webp = libwebp_encode_lossless_rgba(&rgba, w, h);
        let mut out = Vec::new();
        let dims = WebpDecoder::new()
            .decode_to_rgb8(&webp, &mut out)
            .expect("gamut decode");
        assert_eq!((dims.width, dims.height), (w, h), "dims at {w}x{h}");
        assert_eq!(out, rgba_to_rgb(&rgba), "pixel mismatch at {w}x{h}");
    }
}

#[test]
fn libwebp_decodes_gamut_lossless_realistic_and_large() {
    // The reverse direction over the same realistic + large matrix: gamut encodes, the reference
    // decodes back to source — pinning gamut's VP8L *encoder* paths (entropy images, long backward
    // references) as conformant at scale, not just on tiny inputs.
    for &(w, h) in DIMENSIONS.iter().chain(LARGE_DIMENSIONS) {
        let rgb = rgba_to_rgb(&photo_like_rgba(w, h, 0x9a1c));
        assert_gamut_encode_libwebp_decode(&rgb, w, h, &format!("realistic {w}x{h}"));
    }
}

/// Builds a structured YUV 4:2:0 image (real residuals to exercise the transforms/tokens).
fn synthetic_yuv(w: u32, h: u32) -> gamut_color::Yuv420 {
    let (wu, hu) = (w as usize, h as usize);
    let (cw, ch) = (
        gamut_color::Yuv420::chroma_width(w) as usize,
        gamut_color::Yuv420::chroma_height(h) as usize,
    );
    let y = (0..wu * hu)
        .map(|i| ((i * 9 + (i / wu) * 5) & 0xff) as u8)
        .collect();
    let u = (0..cw * ch).map(|i| ((i * 3 + 80) & 0xff) as u8).collect();
    let v = (0..cw * ch).map(|i| ((i * 7 + 150) & 0xff) as u8).collect();
    gamut_color::Yuv420::new(w, h, y, u, v).unwrap()
}

/// B_PRED-favorable content: each 4×4 region carries a different gradient direction, so a single
/// whole-block mode predicts the macroblock poorly and gamut's encoder picks per-subblock `B_PRED`.
fn detailed_yuv(w: u32, h: u32) -> gamut_color::Yuv420 {
    let (wu, hu) = (w as usize, h as usize);
    let (cw, ch) = (
        gamut_color::Yuv420::chroma_width(w) as usize,
        gamut_color::Yuv420::chroma_height(h) as usize,
    );
    let y = (0..wu * hu)
        .map(|i| {
            let (x, yy) = (i % wu, i / wu);
            let v = match (x / 4 + yy / 4) % 4 {
                0 => x * 18,
                1 => yy * 18,
                2 => (x + yy) * 18,
                _ => x.wrapping_sub(yy).wrapping_mul(18),
            };
            (v & 0xff) as u8
        })
        .collect();
    let u = (0..cw * ch).map(|i| ((i * 3) & 0xff) as u8).collect();
    let v = (0..cw * ch).map(|i| ((i * 9 + 70) & 0xff) as u8).collect();
    gamut_color::Yuv420::new(w, h, y, u, v).unwrap()
}

/// Builds a YUV 4:2:0 image with **photographic-like statistics** directly in the YUV domain (so the
/// bit-exact YUV conformance gate stays independent of the RGB↔YCbCr layer, which PR4 pins
/// separately): a smooth luma gradient + low-amplitude detail + hard region edges, with gently
/// varying chroma. RNG-free and `seed`-parameterised, like [`common::photo_like_rgba`].
fn photo_like_yuv(w: u32, h: u32, seed: u32) -> gamut_color::Yuv420 {
    let (wu, hu) = (w as usize, h as usize);
    let (cw, ch) = (
        gamut_color::Yuv420::chroma_width(w) as usize,
        gamut_color::Yuv420::chroma_height(h) as usize,
    );
    let hash = |x: i64, y: i64, k: i64| -> i64 {
        let mut v = x.wrapping_mul(374_761_393)
            ^ y.wrapping_mul(668_265_263)
            ^ k.wrapping_mul(2_654_435_761)
            ^ i64::from(seed).wrapping_mul(2_246_822_519);
        v = (v ^ (v >> 13)).wrapping_mul(1_274_126_177);
        (v ^ (v >> 16)) & 0xff
    };
    let clamp = |v: i64| v.clamp(0, 255) as u8;
    let (wi, hi) = (wu.max(1) as i64, hu.max(1) as i64);
    let y = (0..wu * hu)
        .map(|i| {
            let (x, yy) = ((i % wu) as i64, (i / wu) as i64);
            // Smooth diagonal base + small detail + sharp 8-region steps (B_PRED / loop-filter bait).
            let base = x * 170 / wi + yy * 60 / hi;
            let detail = (hash(x, yy, 0) - 128) / 10;
            let edge = if (x * 4 / wi + yy * 4 / hi) % 2 == 0 {
                28
            } else {
                0
            };
            clamp(base + detail + edge)
        })
        .collect();
    let (cwi, chi) = (cw.max(1) as i64, ch.max(1) as i64);
    let u = (0..cw * ch)
        .map(|i| {
            let (x, yy) = ((i % cw) as i64, (i / cw) as i64);
            clamp(100 + x * 70 / cwi + (hash(x, yy, 1) - 128) / 16)
        })
        .collect();
    let v = (0..cw * ch)
        .map(|i| {
            let (x, yy) = ((i % cw) as i64, (i / cw) as i64);
            clamp(140 + yy * 60 / chi + (hash(x, yy, 2) - 128) / 16)
        })
        .collect();
    gamut_color::Yuv420::new(w, h, y, u, v).unwrap()
}

#[test]
fn gamut_lossy_bpred_matches_libwebp_bit_exact() {
    // Detailed content drives gamut's encoder into per-4×4 B_PRED macroblocks; libwebp must decode the
    // same gamut bitstream to identical YUV — the tier-3 conformance gate for the B_PRED path.
    use common::libwebp_decode_yuv;
    use gamut_riff::write_simple_lossy;
    use gamut_webp::vp8::frame::{decode_frame, encode_frame};

    for &(w, h) in &[(16u32, 16u32), (32, 32), (48, 48), (49, 33), (64, 16)] {
        for &quant_index in &[0u8, 8, 40] {
            let (payload, _) = encode_frame(&detailed_yuv(w, h), quant_index);
            let webp = write_simple_lossy(&payload);
            let lib = libwebp_decode_yuv(&webp);
            let gamut = decode_frame(&payload).expect("gamut decode").to_yuv420();
            assert_eq!((lib.width, lib.height), (w, h), "dims at {w}x{h}");
            assert_eq!(
                gamut.y(),
                lib.y.as_slice(),
                "B_PRED Y mismatch at {w}x{h} q{quant_index}"
            );
            assert_eq!(
                gamut.u(),
                lib.u.as_slice(),
                "B_PRED U mismatch at {w}x{h} q{quant_index}"
            );
            assert_eq!(
                gamut.v(),
                lib.v.as_slice(),
                "B_PRED V mismatch at {w}x{h} q{quant_index}"
            );
        }
    }
}

#[test]
fn gamut_lossy_options_match_libwebp_bit_exact() {
    // The alternative encoder paths — the simple loop filter and quantizer segmentation — must each
    // produce a stream libwebp decodes identically to gamut's decoder (the tier-3 conformance gate).
    use common::libwebp_decode_yuv;
    use gamut_riff::write_simple_lossy;
    use gamut_webp::vp8::frame::{EncodeOptions, decode_frame, encode_frame_filtered};

    let base = EncodeOptions::default();
    let cases = [
        (
            "simple-filter",
            EncodeOptions {
                simple_filter: true,
                ..base
            },
        ),
        (
            "segmented",
            EncodeOptions {
                segmented: true,
                ..base
            },
        ),
        (
            "segmented+simple",
            EncodeOptions {
                simple_filter: true,
                segmented: true,
                ..base
            },
        ),
        (
            "partitions-2",
            EncodeOptions {
                partitions: 2,
                ..base
            },
        ),
        (
            "partitions-4",
            EncodeOptions {
                partitions: 4,
                ..base
            },
        ),
        (
            "partitions-8",
            EncodeOptions {
                partitions: 8,
                ..base
            },
        ),
        (
            "everything",
            EncodeOptions {
                simple_filter: true,
                segmented: true,
                partitions: 4,
            },
        ),
    ];
    for (label, opts) in cases {
        // (33, 145) spans ten macroblock rows, so the eight-partition cases route across every one.
        for &(w, h) in &[(32u32, 32u32), (48, 48), (49, 33), (33, 145)] {
            for &q in &[12u8, 48] {
                let (payload, _) = encode_frame_filtered(&detailed_yuv(w, h), q, opts);
                let webp = write_simple_lossy(&payload);
                let lib = libwebp_decode_yuv(&webp);
                let gamut = decode_frame(&payload).expect("gamut decode").to_yuv420();
                assert_eq!(gamut.y(), lib.y.as_slice(), "{label} Y at {w}x{h} q{q}");
                assert_eq!(gamut.u(), lib.u.as_slice(), "{label} U at {w}x{h} q{q}");
                assert_eq!(gamut.v(), lib.v.as_slice(), "{label} V at {w}x{h} q{q}");
            }
        }
    }
}

#[test]
fn gamut_lossy_yuv_matches_libwebp_bit_exact() {
    // A VP8 bitstream decodes to a deterministic integer YUV, so gamut's own decoder and libwebp must
    // agree bit-for-bit on the same gamut-produced bitstream — the tier-3 conformance gate that pins
    // the encoder to a spec-valid stream. (gamut additionally checks encoder-recon == its own decoder
    // in frame.rs; together these need no pixel tolerance.)
    use common::libwebp_decode_yuv;
    use gamut_riff::write_simple_lossy;
    use gamut_webp::vp8::frame::{decode_frame, encode_frame};

    for &(w, h) in &[
        (16u32, 16u32),
        (32, 32),
        (17, 9),
        (64, 48),
        (80, 16),
        (33, 49),
    ] {
        for &quant_index in &[0u8, 20, 60, 110] {
            let (payload, _) = encode_frame(&synthetic_yuv(w, h), quant_index);
            let webp = write_simple_lossy(&payload);
            let lib = libwebp_decode_yuv(&webp);
            let gamut = decode_frame(&payload).expect("gamut decode").to_yuv420();
            assert_eq!((lib.width, lib.height), (w, h), "dims at {w}x{h}");
            assert_eq!(
                gamut.y(),
                lib.y.as_slice(),
                "Y mismatch at {w}x{h} q{quant_index}"
            );
            assert_eq!(
                gamut.u(),
                lib.u.as_slice(),
                "U mismatch at {w}x{h} q{quant_index}"
            );
            assert_eq!(
                gamut.v(),
                lib.v.as_slice(),
                "V mismatch at {w}x{h} q{quant_index}"
            );
        }
    }
}

#[test]
fn gamut_lossy_yuv_realistic_and_large_matches_libwebp() {
    // The same tier-3 conformance gate as above, but on photographic-like YUV and on canvases far
    // larger than the small synthetic frames: macroblock grids spanning dozens of rows (8-partition
    // routing across all of them) and realistic residual/token distributions. gamut's own decoder and
    // libwebp must still agree bit-for-bit on the gamut-produced stream.
    use common::libwebp_decode_yuv;
    use gamut_riff::write_simple_lossy;
    use gamut_webp::vp8::frame::{decode_frame, encode_frame};

    // Large sizes span many MB rows (768/16 = 48, so 8-partition routing touches every row) and an
    // awkward width (513×97). gamut's encoder is fast enough here that the full range stays cheap.
    let dims = [
        (32u32, 32u32),
        (64, 48),
        (256, 256),
        (384, 288),
        (640, 480),
        (1024, 768),
        (513, 97),
    ];
    for &(w, h) in &dims {
        for &quant_index in &[12u8, 56] {
            let (payload, _) = encode_frame(&photo_like_yuv(w, h, 0x7e57), quant_index);
            let webp = write_simple_lossy(&payload);
            let lib = libwebp_decode_yuv(&webp);
            let gamut = decode_frame(&payload).expect("gamut decode").to_yuv420();
            assert_eq!((lib.width, lib.height), (w, h), "dims at {w}x{h}");
            assert_eq!(gamut.y(), lib.y.as_slice(), "Y at {w}x{h} q{quant_index}");
            assert_eq!(gamut.u(), lib.u.as_slice(), "U at {w}x{h} q{quant_index}");
            assert_eq!(gamut.v(), lib.v.as_slice(), "V at {w}x{h} q{quant_index}");
        }
    }
}

/// Extracts the `VP8 ` (lossy) chunk payload from a RIFF/WebP file.
fn vp8_payload(webp: &[u8]) -> Vec<u8> {
    use gamut_riff::{RiffReader, WebpChunkId};
    RiffReader::new(webp)
        .expect("riff")
        .filter_map(Result::ok)
        .find(|c| matches!(WebpChunkId::from(c.fourcc), WebpChunkId::Vp8))
        .expect("VP8 chunk")
        .payload
        .to_vec()
}

#[test]
fn gamut_decodes_libwebp_lossy_bit_exact() {
    // The reverse-direction conformance gate: a real libwebp lossy encoder emits VP8 streams using its
    // own loop filter, segmentation, token-probability updates, and skip choices. gamut's native
    // decoder must reproduce libwebp's own YUV output bit-for-bit — proving it handles the full
    // feature surface a production encoder actually emits, not just gamut's own streams.
    use common::{libwebp_decode_yuv, libwebp_encode_lossy_rgba};

    for &(w, h) in &[
        (16u32, 16u32),
        (32, 32),
        (64, 48),
        (49, 33),
        (80, 17),
        (255, 3),
    ] {
        for q in [6.0f32, 35.0, 70.0, 100.0] {
            let rgba = pattern_rgba(w, h);
            let webp = libwebp_encode_lossy_rgba(&rgba, w, h, q);
            let gamut = gamut_webp::vp8::frame::decode_frame(&vp8_payload(&webp))
                .expect("gamut decode")
                .to_yuv420();
            let lib = libwebp_decode_yuv(&webp);
            assert_eq!((lib.width, lib.height), (w, h), "dims at {w}x{h}");
            assert_eq!(gamut.y(), lib.y.as_slice(), "Y at {w}x{h} q{q}");
            assert_eq!(gamut.u(), lib.u.as_slice(), "U at {w}x{h} q{q}");
            assert_eq!(gamut.v(), lib.v.as_slice(), "V at {w}x{h} q{q}");
        }
    }
}

#[test]
fn gamut_decodes_libwebp_lossy_realistic_and_large() {
    // The reverse direction at scale: libwebp lossy-encodes photographic RGBA over a lossy-appropriate
    // small matrix plus the full LARGE_DIMENSIONS (up to 1024×768), and gamut's decoder must reproduce
    // libwebp's own YUV bit-for-bit — pinning the decode paths (per-segment filter levels, probability
    // updates, partitioning) on frames far larger and more varied than the tiny synthetic inputs.
    use common::{libwebp_decode_yuv, libwebp_encode_lossy_rgba};

    let small = [
        (16u32, 16u32),
        (32, 32),
        (64, 48),
        (49, 33),
        (80, 17),
        (255, 3),
    ];
    for &(w, h) in small.iter().chain(LARGE_DIMENSIONS) {
        for q in [20.0f32, 80.0] {
            let rgba = photo_like_rgba(w, h, 0x1d0f);
            let webp = libwebp_encode_lossy_rgba(&rgba, w, h, q);
            let gamut = gamut_webp::vp8::frame::decode_frame(&vp8_payload(&webp))
                .expect("gamut decode")
                .to_yuv420();
            let lib = libwebp_decode_yuv(&webp);
            assert_eq!((lib.width, lib.height), (w, h), "dims at {w}x{h}");
            assert_eq!(gamut.y(), lib.y.as_slice(), "Y at {w}x{h} q{q}");
            assert_eq!(gamut.u(), lib.u.as_slice(), "U at {w}x{h} q{q}");
            assert_eq!(gamut.v(), lib.v.as_slice(), "V at {w}x{h} q{q}");
        }
    }
}

#[test]
fn gamut_decodes_libwebp_lossy_forced_features_bit_exact() {
    // The one-shot `WebPEncodeRGBA` the other reverse-direction tests use can't independently reach
    // the VP8 feature surface — it runs at a fixed method with the complex loop filter and libwebp's
    // default segmentation. Drive libwebp's *advanced* encoder to force each knob in turn (simple vs
    // complex filter, 1..=4 segments, low/high effort, filter strength, then several at once) and
    // require gamut's decoder to reproduce libwebp's own YUV bit-for-bit on every variant — pinning
    // the decoder against streams a production encoder can emit but cwebp's defaults rarely do.
    use common::{LibwebpLossyConfig, libwebp_decode_yuv, libwebp_encode_lossy_rgba_config};

    let base = LibwebpLossyConfig {
        quality: 75.0,
        filter_type: 1,
        segments: 1,
        method: 4,
        filter_strength: 60,
    };

    // Guard: prove the advanced config actually reaches the encoder. This is a *conformance* gate
    // (gamut must match whatever libwebp emits), so it would still pass if the forcing were silently
    // ignored — but then it would add nothing over the default-config reverse oracle. Require distinct
    // feature settings to produce distinct streams for the same input. (Token-partition count is *not*
    // guarded here: libwebp's encoder ignores `config.partitions` and always writes one partition —
    // see `LibwebpLossyConfig` — so that path is covered forward in `gamut_lossy_options_*` instead.)
    {
        let (w, h) = (64u32, 48);
        let rgba = photo_like_rgba(w, h, 0x00c0_ffee);
        let one = libwebp_encode_lossy_rgba_config(&rgba, w, h, &base);
        let simple = libwebp_encode_lossy_rgba_config(
            &rgba,
            w,
            h,
            &LibwebpLossyConfig {
                filter_type: 0,
                ..base
            },
        );
        assert_ne!(
            one, simple,
            "switching to the simple loop filter must change the stream"
        );
        let multi_seg = libwebp_encode_lossy_rgba_config(
            &rgba,
            w,
            h,
            &LibwebpLossyConfig {
                segments: 4,
                ..base
            },
        );
        assert_ne!(
            one, multi_seg,
            "forcing four segments must change the stream"
        );
    }

    let mut cases: Vec<(String, LibwebpLossyConfig)> = Vec::new();
    for filter_type in [0, 1] {
        cases.push((
            format!("filter_type={filter_type}"),
            LibwebpLossyConfig {
                filter_type,
                ..base
            },
        ));
    }
    for segments in [1, 2, 3, 4] {
        cases.push((
            format!("segments={segments}"),
            LibwebpLossyConfig { segments, ..base },
        ));
    }
    for method in [0, 3, 6] {
        cases.push((
            format!("method={method}"),
            LibwebpLossyConfig { method, ..base },
        ));
    }
    for filter_strength in [0, 30] {
        cases.push((
            format!("filter_strength={filter_strength}"),
            LibwebpLossyConfig {
                filter_strength,
                ..base
            },
        ));
    }
    cases.push((
        "combined".into(),
        LibwebpLossyConfig {
            filter_type: 0,
            segments: 4,
            method: 6,
            filter_strength: 20,
            ..base
        },
    ));

    // (128, 96) spans six MB rows, exercising per-segment filter levels across many rows.
    for &(w, h) in &[(32u32, 32u32), (64, 48), (49, 33), (128, 96)] {
        let rgba = photo_like_rgba(w, h, 0x00c0_ffee);
        for (label, cfg) in &cases {
            let webp = libwebp_encode_lossy_rgba_config(&rgba, w, h, cfg);
            let gamut = gamut_webp::vp8::frame::decode_frame(&vp8_payload(&webp))
                .expect("gamut decode")
                .to_yuv420();
            let lib = libwebp_decode_yuv(&webp);
            assert_eq!((lib.width, lib.height), (w, h), "dims {label} at {w}x{h}");
            assert_eq!(gamut.y(), lib.y.as_slice(), "Y {label} at {w}x{h}");
            assert_eq!(gamut.u(), lib.u.as_slice(), "U {label} at {w}x{h}");
            assert_eq!(gamut.v(), lib.v.as_slice(), "V {label} at {w}x{h}");
        }
    }
}

#[test]
fn libwebp_decodes_gamut_lossy_alpha_exactly() {
    // gamut encodes lossy color plus a raw `ALPH` alpha plane in an extended (`VP8X`) file; libwebp
    // must recover the exact alpha (alpha is lossless). The lossy color is not compared.
    use common::libwebp_decode_rgba;

    for &(w, h) in &[(16u32, 16u32), (32, 24), (17, 9), (49, 33)] {
        let rgba: Vec<u8> = (0..(w * h) as usize)
            .flat_map(|i| {
                let (x, y) = (i as u32 % w, i as u32 / w);
                [
                    (x * 7) as u8,
                    (y * 9) as u8,
                    (x ^ y) as u8,
                    ((x * 11 + y * 5) & 0xff) as u8,
                ]
            })
            .collect();
        let mut file = Vec::new();
        WebpEncoder::lossy(70)
            .encode_rgba8(
                &rgba,
                Dimensions {
                    width: w,
                    height: h,
                },
                &mut file,
            )
            .expect("gamut rgba encode");
        let decoded = libwebp_decode_rgba(&file);
        assert_eq!((decoded.width, decoded.height), (w, h), "dims at {w}x{h}");
        let lib_alpha: Vec<u8> = decoded.rgba.chunks_exact(4).map(|p| p[3]).collect();
        let src_alpha: Vec<u8> = rgba.chunks_exact(4).map(|p| p[3]).collect();
        assert_eq!(
            lib_alpha, src_alpha,
            "libwebp must recover gamut's exact alpha at {w}x{h}"
        );
    }
}

#[test]
fn gamut_decodes_libwebp_lossy_alpha_exactly() {
    // libwebp encodes lossy color plus a *compressed* (C=1) `ALPH` plane (what cwebp emits); gamut's
    // decoder must recover the exact alpha — the reverse-direction gate for the lossless-alpha path.
    use common::libwebp_encode_lossy_rgba;

    for &(w, h) in &[(16u32, 16u32), (32, 24), (49, 33), (80, 17)] {
        let rgba: Vec<u8> = (0..(w * h) as usize)
            .flat_map(|i| {
                let (x, y) = (i as u32 % w, i as u32 / w);
                [
                    (x * 7) as u8,
                    (y * 9) as u8,
                    (x ^ y) as u8,
                    ((x * 11 + y * 5) & 0xff) as u8,
                ]
            })
            .collect();
        let webp = libwebp_encode_lossy_rgba(&rgba, w, h, 75.0);
        let mut out = Vec::new();
        let d = WebpDecoder::new()
            .decode_to_rgba8(&webp, &mut out)
            .expect("gamut decode libwebp lossy+alpha");
        assert_eq!(
            d,
            Dimensions {
                width: w,
                height: h
            }
        );
        let dec_alpha: Vec<u8> = out.chunks_exact(4).map(|p| p[3]).collect();
        let src_alpha: Vec<u8> = rgba.chunks_exact(4).map(|p| p[3]).collect();
        assert_eq!(
            dec_alpha, src_alpha,
            "gamut must recover libwebp's exact alpha at {w}x{h}"
        );
    }
}
