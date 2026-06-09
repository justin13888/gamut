//! Bit-exact reconstruction cross-check for the lossy intra path (P6 keystone).
//!
//! The encoder maintains a reconstruction buffer that must equal, sample for sample, what a
//! conformant decoder produces. This test encodes a lossy still, decodes the raw AV1 OBU stream
//! (a Section-5 low-overhead stream — each OBU carries its size) with the real `dav1d` decoder, and
//! asserts the decoded planes equal the encoder's exported reconstruction byte-for-byte.
//!
//! `dav1d` is linked in from the `third_party/dav1d` submodule via the `dav1d-oracle`
//! dev-dependency, so the check is hermetic and always runs — it never depends on a `dav1d` binary
//! being installed on the host. Building these tests therefore needs meson/ninja/nasm and the
//! checked-out submodule (`git submodule update --init --recursive`).

use gamut_av1::encode_still_intra;
use gamut_color::Planar8;

/// Builds identity planes (Y=G, U=B, V=R) from an RGB generator.
fn planes(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> Planar8 {
    let mut rgb = vec![0u8; (w * h * 3) as usize];
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 3) as usize;
            rgb[i..i + 3].copy_from_slice(&f(x, y));
        }
    }
    Planar8::from_rgb8_identity(&rgb, w, h).unwrap()
}

/// Encodes `planes` at `qindex`, decodes the OBU stream with dav1d, and asserts the decoded planes
/// equal the encoder's reconstruction.
fn check(planes: &Planar8, qindex: u8) {
    let (still, recon) = encode_still_intra(planes, qindex).unwrap();
    let (w, h) = (recon.width as usize, recon.height as usize);

    let dir = std::env::temp_dir();
    // A globally-unique stamp: tests run in parallel and several share (w, h, qindex), so the
    // process id alone would collide on the temp paths and race.
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let stamp = format!(
        "{}_{w}x{h}_q{qindex}_{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    );
    let obu_path = dir.join(format!("gamut_recon_{stamp}.obu"));
    let y4m_path = dir.join(format!("gamut_recon_{stamp}.y4m"));
    // A standalone Section-5 OBU stream for dav1d needs a temporal-delimiter OBU first (AVIF omits
    // it inside the container). TD = obu_type 2, has_size_field, empty payload.
    let mut stream = vec![0x12u8, 0x00];
    stream.extend_from_slice(&still.obus);

    let decoded = dav1d_oracle::decode_obu(&stream)
        .unwrap_or_else(|e| panic!("dav1d decode failed for {w}x{h} q{qindex}: {e}"));
    assert_eq!(
        (decoded.width as usize, decoded.height as usize),
        (w, h),
        "decoded dimensions differ from reconstruction for {w}x{h} q{qindex}"
    );

    for (p, (dec, enc)) in decoded.planes.iter().zip(&recon.planes).enumerate() {
        assert_eq!(
            dec, enc,
            "plane {p} mismatch (decoder vs encoder reconstruction) for {w}x{h} q{qindex}"
        );
    }
}

#[test]
fn lossy_reconstruction_matches_dav1d() {
    // A photographic-ish gradient with texture: exercises non-trivial residuals, DC prediction
    // across block boundaries, and the all-zero (txb_skip) path on flat regions.
    let texture = |x: u32, y: u32| {
        let r = (x.wrapping_mul(3).wrapping_add(y) % 256) as u8;
        let g = ((x + y.wrapping_mul(2)) % 256) as u8;
        let b = (128 + ((x ^ y) % 64)) as u8;
        [r, g, b]
    };
    // qindex stays in 1..=20 so the coefficient-CDF quantizer context is 0 (matches the static CDFs).
    for &q in &[4u8, 12, 20] {
        for &(w, h) in &[(8, 8), (17, 13), (32, 32), (64, 48), (100, 70)] {
            check(&planes(w, h, texture), q);
        }
    }
}

#[test]
fn lossy_reconstruction_matches_dav1d_all_qctx() {
    // The same textured content, but at quantizers spanning every coefficient-CDF quantizer
    // context: qctx 1 (21..=60), qctx 2 (61..=120) and qctx 3 (121..=255). A wrong CDF table makes
    // the arithmetic decode diverge, so dav1d byte-equality is a hard correctness gate per qctx.
    let texture = |x: u32, y: u32| {
        let r = (x.wrapping_mul(5).wrapping_add(y.wrapping_mul(3)) % 256) as u8;
        let g = ((x.wrapping_add(y).wrapping_mul(2)) % 256) as u8;
        let b = (64 + ((x.wrapping_mul(7) ^ y) % 128)) as u8;
        [r, g, b]
    };
    // One representative qindex per context boundary, plus the extremes.
    for &q in &[21u8, 40, 60, 61, 90, 120, 121, 200, 255] {
        for &(w, h) in &[(8, 8), (17, 13), (40, 24), (100, 70)] {
            check(&planes(w, h, texture), q);
        }
    }
}

#[test]
fn tx_type_selection_matches_dav1d() {
    // Content engineered so the encoder's per-block transform-type search picks non-DCT_DCT types
    // from TX_SET_INTRA_2 (IDTX on sharp screen-content edges, ADST on directional ramps). dav1d
    // must decode whatever type was signaled to the encoder's reconstruction byte-for-byte, so this
    // exercises the ADST/IDTX inverse transforms end-to-end through the real decoder.
    let screen = |x: u32, y: u32| {
        // 1-pixel-wide vertical bars + a diagonal ramp: high-frequency, impulse-like residuals.
        let bar = if x.is_multiple_of(2) { 235 } else { 20 } as u8;
        let ramp = ((x.wrapping_add(y)).wrapping_mul(9) % 256) as u8;
        let diag = if (x + y).is_multiple_of(7) { 250 } else { 40 } as u8;
        [bar, ramp, diag]
    };
    for &q in &[10u8, 32, 80, 160] {
        for &(w, h) in &[(8, 8), (16, 16), (37, 21), (64, 40)] {
            check(&planes(w, h, screen), q);
        }
    }
}

#[test]
fn directional_modes_match_dav1d() {
    // Strong vertical, horizontal and diagonal structure so the mode search picks the directional
    // modes (V/H/D135/D113/D157). dav1d must decode each signaled angle to the encoder
    // reconstruction byte-for-byte, exercising the directional prediction process end-to-end.
    let directional = |x: u32, y: u32| {
        let vert = if (x / 2).is_multiple_of(2) { 210 } else { 30 } as u8; // vertical bars
        let horiz = if (y / 2).is_multiple_of(2) { 200 } else { 40 } as u8; // horizontal bars
        let diag = (((x + y) * 16) % 256) as u8; // 45° ramp
        [vert, horiz, diag]
    };
    for &q in &[8u8, 28, 96, 180] {
        for &(w, h) in &[(8, 8), (16, 16), (37, 21), (64, 40)] {
            check(&planes(w, h, directional), q);
        }
    }
}

#[test]
fn filter_intra_modes_match_dav1d() {
    // Mixed flat-plus-fine-texture content: the encoder's per-block search picks recursive
    // filter-intra (§7.11.2.3, signaled as a DC_PRED block + `use_filter_intra` + a
    // `filter_intra_mode`) on many blocks. dav1d must run the same recursive predictor and reach the
    // encoder's reconstruction byte-for-byte. (The encoder selects all five filter modes on this
    // content, so it covers FILTER_DC/V/H/D157/PAETH through the real decoder.)
    let textured = |x: u32, y: u32| {
        let r = (x.wrapping_mul(3).wrapping_add(y) % 256) as u8;
        let g = ((x + y.wrapping_mul(2)) % 256) as u8;
        let b = (128 + ((x ^ y) % 64)) as u8;
        [r, g, b]
    };
    for &q in &[6u8, 24, 88, 170] {
        for &(w, h) in &[(8, 8), (16, 16), (37, 21), (64, 40)] {
            check(&planes(w, h, textured), q);
        }
    }
}

#[test]
fn cfl_chroma_from_luma_matches_dav1d() {
    // Chroma that tracks the luma high-frequency: U falls as luma rises (negative alpha), V rises
    // with it (positive alpha). The encoder's per-block CfL search then signals uv_mode = UV_CFL_PRED
    // with non-zero CflAlphaU/CflAlphaV, and dav1d must run §7.11.5 chroma-from-luma to the encoder's
    // reconstruction byte-for-byte — exercising both alpha signs and read_cfl_alphas end-to-end.
    let cfl = |x: u32, y: u32| {
        let base = ((x.wrapping_mul(7).wrapping_add(y.wrapping_mul(5))) % 200) as i32 + 28; // luma
        let g = base as u8;
        let r = (base / 2 + 100).clamp(0, 255) as u8; // tracks +luma
        let b = (220 - base / 2).clamp(0, 255) as u8; // tracks -luma
        [r, g, b]
    };
    for &q in &[6u8, 24, 88, 170] {
        for &(w, h) in &[(8, 8), (16, 16), (37, 21), (64, 40)] {
            check(&planes(w, h, cfl), q);
        }
    }
}

#[test]
fn deblock_matches_dav1d() {
    // Block-aligned flat tiles with moderate steps between them: after quantization the 4×4 block
    // boundaries carry exactly the small discontinuities the deblocking loop filter (§7.14) smooths.
    // The encoder applies the filter to its reconstruction and dav1d applies it on decode, so the
    // byte-for-byte match validates the narrow-filter math, masks, and the vertical-then-horizontal
    // pass ordering across the full quantizer (and hence loop-filter-level) range.
    let tiles = |x: u32, y: u32| {
        let step = ((x / 4 + y / 4) % 6) as u8; // changes every 4 px ⇒ on the block grid
        let v = 60u8.wrapping_add(step.wrapping_mul(18));
        [
            v,
            v.wrapping_add(20),
            200u8.wrapping_sub(step.wrapping_mul(12)),
        ]
    };
    for &q in &[16u8, 48, 110, 200] {
        for &(w, h) in &[(8, 8), (16, 16), (35, 23), (64, 40)] {
            check(&planes(w, h, tiles), q);
        }
    }
}

#[test]
fn cdef_matches_dav1d() {
    if !dav1d_available() {
        eprintln!("skipping recon_dav1d: dav1d not installed");
        return;
    }
    // Strong directional structure (diagonal/edged content) gives the CDEF direction search (§7.15.2)
    // distinct per-8×8 directions and non-trivial variance, so the primary+secondary deringing filter
    // (§7.15.3) actually fires. The encoder runs deblock → CDEF on its reconstruction and dav1d does
    // the same on decode, so byte-equality validates the direction search, constrain, taps, and the
    // out-of-frame sample availability — at quantizers spanning the signaled CDEF strength range.
    let edged = |x: u32, y: u32| {
        let d = ((x + y) % 16) as u8; // 45° structure on the 8×8 CDEF grid
        let r = if d < 8 { 40 } else { 210 };
        let g = (30u8).wrapping_add(d.wrapping_mul(13));
        let b = if (x.wrapping_sub(y)) % 12 < 6 {
            70
        } else {
            190
        };
        [r, g, b]
    };
    for &q in &[32u8, 80, 128, 220] {
        for &(w, h) in &[(8, 8), (16, 16), (35, 23), (64, 40)] {
            check(&planes(w, h, edged), q);
        }
    }
}

#[test]
fn flat_lossy_reconstruction_matches_dav1d() {
    // A solid color: every residual quantizes to zero, so the reconstruction is the DC prediction
    // chain — a clean test that prediction-from-reconstruction tracks the decoder exactly.
    check(&planes(48, 40, |_, _| [200, 100, 50]), 16);
}
