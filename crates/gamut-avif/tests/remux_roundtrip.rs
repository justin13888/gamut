//! Hermetic remux round-trip: `gamut-isobmff` parses a *foreign* (libavif-encoded) AVIF, re-writes
//! it normalised, and libavif decodes the re-muxed container to identical pixels.
//!
//! This guards a guarantee `gamut-isobmff`'s own `read(&write) == model` check cannot: that
//! re-serialising a container this crate did **not** write preserves everything that affects
//! rendering (the coded payload is carried verbatim through the box model). libavif (dav1d backend)
//! is linked from the `third_party/libavif` + `third_party/dav1d` submodules via the
//! `libavif-oracle` dev-dependency, so — like `decode_roundtrip.rs` — the check is hermetic but
//! needs cmake/meson/ninja/nasm and the checked-out submodules.

use std::path::PathBuf;

/// A real 4:4:4 libavif corpus file. The oracle's `decode_avif` only handles 4:4:4 (the form gamut
/// emits), so a 4:2:0 fixture would be rejected before the pixel comparison.
fn corpus_444() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../third_party/libavif/tests/data/io/cosmos1650_yuv444_10bpc_p3pq.avif")
}

#[test]
fn remux_preserves_decoded_pixels() {
    let src = std::fs::read(corpus_444()).expect("read the corpus fixture");

    // gamut-isobmff parses the foreign container and re-serialises it (normalised box versions,
    // single-extent mdat), never touching the opaque coded payload.
    let model = gamut_isobmff::read(&src).expect("gamut-isobmff reads the foreign container");
    let remuxed = gamut_isobmff::write(&model).expect("gamut-isobmff writes the container");
    assert_ne!(
        remuxed, src,
        "the writer normalises, so the container bytes are expected to differ"
    );

    // The reference decoder must reproduce identical pixels from both containers.
    let original = libavif_oracle::decode_avif(&src).expect("libavif decodes the original");
    let round_tripped =
        libavif_oracle::decode_avif(&remuxed).expect("libavif decodes the re-muxed container");

    assert_eq!(
        (original.width, original.height, original.bit_depth),
        (
            round_tripped.width,
            round_tripped.height,
            round_tripped.bit_depth
        ),
    );
    assert_eq!(
        original.planes, round_tripped.planes,
        "decoded pixels differ after remux — the container round-trip lost rendering data"
    );
}
