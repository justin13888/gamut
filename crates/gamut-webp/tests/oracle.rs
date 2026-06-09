//! Differential oracle baseline (libwebp).
//!
//! This proves the libwebp oracle harness — the `libwebp-sys2` FFI wrappers in `common` plus the
//! statically linked libwebp build — works end-to-end before any gamut codec exists. As the VP8L
//! (M0) and VP8 (M2) paths land, this file gains the real differential checks:
//!   - gamut encode → libwebp decode == source (lossless ⇒ bit-exact);
//!   - libwebp encode → gamut decode == libwebp decode (validates the native decoder).

mod common;

use common::{libwebp_decode_rgba, libwebp_encode_lossless_rgba, libwebp_get_info, pattern_rgba};
use gamut_webp::WebpDecoder;

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
fn gamut_decoder_recognizes_libwebp_lossless_output() {
    // Cross-check the container layer: gamut parses a real libwebp file and routes it to the
    // (not-yet-implemented) VP8L decoder. This becomes a full pixel-equality check once VP8L lands.
    let rgba = pattern_rgba(12, 10);
    let webp = libwebp_encode_lossless_rgba(&rgba, 12, 10);
    let mut out = Vec::new();
    let msg = WebpDecoder::new()
        .decode_to_rgb8(&webp, &mut out)
        .unwrap_err()
        .to_string();
    assert!(
        msg.contains("VP8L"),
        "gamut should route libwebp output to VP8L, got: {msg}"
    );
}
