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

use common::{libwebp_decode_rgba, libwebp_encode_lossless_rgba, libwebp_get_info, pattern_rgba};
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
