//! End-to-end correctness: encode → decode with libavif → compare planes to the source.
//!
//! This is the authoritative container check: a real AVIF reader must parse gamut's container and
//! reproduce the encoder's pixels. libavif (dav1d backend) is linked in from the
//! `third_party/libavif` + `third_party/dav1d` submodules via the `libavif-oracle` dev-dependency,
//! so the check is hermetic and always runs — it never depends on an `avifdec` binary being
//! installed. Building these tests therefore needs cmake/meson/ninja/nasm and the checked-out
//! submodules (`git submodule update --init --recursive`).

use gamut_avif::AvifEncoder;
use gamut_core::{Dimensions, EncodeImage, ImageRef, Rgb8};

/// Source RGB pattern (structure + variation to exercise nonzero coefficients).
fn rgb_at(x: u32, y: u32) -> (u8, u8, u8) {
    (
        ((x * 7 + y * 3) & 0xff) as u8,
        ((x * x + y) & 0xff) as u8,
        ((x ^ (y * 5)) & 0xff) as u8,
    )
}

/// Builds the interleaved RGB source buffer for a `w`×`h` frame.
fn source_rgb(w: u32, h: u32) -> Vec<u8> {
    let mut rgb = vec![0u8; (w * h * 3) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 3) as usize;
            let (r, g, b) = rgb_at(x, y);
            rgb[i] = r;
            rgb[i + 1] = g;
            rgb[i + 2] = b;
        }
    }
    rgb
}

fn roundtrip(w: u32, h: u32) {
    let rgb = source_rgb(w, h);

    let mut avif = Vec::new();
    AvifEncoder::new()
        .encode_image(
            ImageRef::<Rgb8>::new(
                &rgb,
                Dimensions {
                    width: w,
                    height: h,
                },
            )
            .unwrap(),
            &mut avif,
        )
        .unwrap();

    let decoded = libavif_oracle::decode_avif(&avif)
        .unwrap_or_else(|e| panic!("libavif decode failed for {w}x{h}: {e}"));
    assert_eq!((decoded.width, decoded.height), (w, h));
    let [yp, up, vp] = &decoded.planes;

    // Identity matrix mapping: Y=G, U=B, V=R.
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let (r, g, b) = rgb_at(x, y);
            assert_eq!(yp[i], u16::from(g), "Y!=G at ({x},{y}) in {w}x{h}");
            assert_eq!(up[i], u16::from(b), "U!=B at ({x},{y}) in {w}x{h}");
            assert_eq!(vp[i], u16::from(r), "V!=R at ({x},{y}) in {w}x{h}");
        }
    }
}

#[test]
fn lossless_roundtrip_via_libavif() {
    // Tiny, non-aligned (edge padding + forced partition splits), single-SB, and multi-SB frames.
    for (w, h) in [
        (1, 1),
        (8, 8),
        (17, 13),
        (31, 31),
        (64, 64),
        (100, 80),
        (200, 150),
    ] {
        roundtrip(w, h);
    }
}

#[test]
fn lossy_roundtrip_via_libavif() {
    // For lossy coding the decoded image is not the source, but it must equal the AV1 encoder's
    // own reconstruction byte-for-byte: libavif runs a conformant decoder (dav1d) over the OBUs the
    // container carries, so this validates the whole container + lossy AV1 path end-to-end.
    for &q in &[6u8, 24, 64, 150] {
        for &(w, h) in &[(8, 8), (17, 13), (40, 24), (100, 80)] {
            let rgb = source_rgb(w, h);

            let mut avif = Vec::new();
            AvifEncoder::new()
                .with_qindex(q)
                .encode_image(
                    ImageRef::<Rgb8>::new(
                        &rgb,
                        Dimensions {
                            width: w,
                            height: h,
                        },
                    )
                    .unwrap(),
                    &mut avif,
                )
                .unwrap();

            // The AV1 layer's reconstruction (the exact decoder output) for the same input.
            let planes = gamut_color::Planar8::from_rgb8_identity(&rgb, w, h).unwrap();
            let (_, recon) = gamut_av1::encode_still_intra(&planes, q).unwrap();

            let decoded = libavif_oracle::decode_avif(&avif)
                .unwrap_or_else(|e| panic!("libavif decode failed for {w}x{h} q{q}: {e}"));
            // Identity matrix: decoded Y/U/V planes are the AV1 recon planes 0/1/2.
            for (p, (d, r)) in decoded.planes.iter().zip(&recon.planes).enumerate() {
                assert_eq!(
                    d, r,
                    "plane {p} mismatch (libavif vs AV1 recon) for {w}x{h} q{q}"
                );
            }
        }
    }
}

#[test]
fn orientation_transforms_roundtrip_via_libavif() {
    // `irot`/`imir` are display-time transforms marked *essential*: a conformant reader must parse
    // and honour them or reject the file, so libavif decoding successfully proves they are
    // well-formed MIAF properties. libavif (default settings) does not bake the transform into the
    // returned samples, so the stored planes are unchanged — the lossless pixels still round-trip.
    let (w, h) = (24u32, 16u32);
    let rgb = source_rgb(w, h);
    for (rot, mir) in [
        (1u8, None),
        (0u8, Some(1u8)),
        (3u8, Some(0u8)),
        (2u8, Some(1u8)),
    ] {
        let mut enc = AvifEncoder::new().with_rotation_ccw(rot);
        if let Some(axis) = mir {
            enc = enc.with_mirror(axis);
        }
        let mut avif = Vec::new();
        enc.encode_image(
            ImageRef::<Rgb8>::new(
                &rgb,
                Dimensions {
                    width: w,
                    height: h,
                },
            )
            .unwrap(),
            &mut avif,
        )
        .unwrap();

        let decoded = libavif_oracle::decode_avif(&avif)
            .unwrap_or_else(|e| panic!("libavif rejected irot={rot} imir={mir:?}: {e}"));
        assert_eq!(
            (decoded.width, decoded.height),
            (w, h),
            "coded dims unchanged"
        );
        let [yp, up, vp] = &decoded.planes;
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) as usize;
                let (r, g, b) = rgb_at(x, y);
                assert_eq!(
                    yp[i],
                    u16::from(g),
                    "Y at ({x},{y}) irot={rot} imir={mir:?}"
                );
                assert_eq!(up[i], u16::from(b));
                assert_eq!(vp[i], u16::from(r));
            }
        }
    }
}
