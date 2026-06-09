//! Bit-exact reconstruction cross-check for the lossy intra path (P6 keystone).
//!
//! The encoder maintains a reconstruction buffer that must equal, sample for sample, what a
//! conformant decoder produces. This test encodes a lossy still, decodes the raw AV1 OBU stream
//! with `dav1d` (a Section-5 low-overhead OBU stream — each OBU carries its size), and asserts the
//! decoded planes equal the encoder's exported reconstruction byte-for-byte. It is skipped when
//! `dav1d` is not installed.

use std::io::Write;
use std::process::Command;

use gamut_av1::encode_still_intra;
use gamut_color::Planar8;

fn dav1d_available() -> bool {
    Command::new("dav1d")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

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

/// Parses a `C444` 8-bit Y4M into three `width * height` planes.
fn parse_y4m(data: &[u8], width: usize, height: usize) -> [Vec<u8>; 3] {
    let header_end = data.iter().position(|&b| b == b'\n').expect("y4m header");
    let rest = &data[header_end + 1..];
    // FRAME header line.
    let frame_end = rest.iter().position(|&b| b == b'\n').expect("y4m frame");
    let body = &rest[frame_end + 1..];
    let n = width * height;
    assert!(body.len() >= 3 * n, "y4m body too short");
    [
        body[0..n].to_vec(),
        body[n..2 * n].to_vec(),
        body[2 * n..3 * n].to_vec(),
    ]
}

/// Encodes `planes` at `qindex`, decodes the OBU stream with dav1d, and asserts the decoded planes
/// equal the encoder's reconstruction.
fn check(planes: &Planar8, qindex: u8) {
    let (still, recon) = encode_still_intra(planes, qindex).unwrap();
    let (w, h) = (recon.width as usize, recon.height as usize);

    let dir = std::env::temp_dir();
    let stamp = format!("{}_{w}x{h}_q{qindex}", std::process::id());
    let obu_path = dir.join(format!("gamut_recon_{stamp}.obu"));
    let y4m_path = dir.join(format!("gamut_recon_{stamp}.y4m"));
    // A standalone Section-5 OBU stream for dav1d needs a temporal-delimiter OBU first (AVIF omits
    // it inside the container). TD = obu_type 2, has_size_field, empty payload.
    let mut stream = vec![0x12u8, 0x00];
    stream.extend_from_slice(&still.obus);
    std::fs::File::create(&obu_path)
        .unwrap()
        .write_all(&stream)
        .unwrap();

    let out = Command::new("dav1d")
        .args(["-q", "-i"])
        .arg(&obu_path)
        .arg("-o")
        .arg(&y4m_path)
        .output()
        .expect("run dav1d");
    assert!(
        out.status.success(),
        "dav1d failed for {w}x{h} q{qindex}: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let y4m = std::fs::read(&y4m_path).unwrap();
    let decoded = parse_y4m(&y4m, w, h);
    let _ = std::fs::remove_file(&obu_path);
    let _ = std::fs::remove_file(&y4m_path);

    for (p, (dec, enc)) in decoded.iter().zip(&recon.planes).enumerate() {
        assert_eq!(
            dec, enc,
            "plane {p} mismatch (decoder vs encoder reconstruction) for {w}x{h} q{qindex}"
        );
    }
}

#[test]
fn lossy_reconstruction_matches_dav1d() {
    if !dav1d_available() {
        eprintln!("skipping recon_dav1d: dav1d not installed");
        return;
    }
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
    if !dav1d_available() {
        eprintln!("skipping recon_dav1d: dav1d not installed");
        return;
    }
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
    if !dav1d_available() {
        eprintln!("skipping recon_dav1d: dav1d not installed");
        return;
    }
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
    if !dav1d_available() {
        eprintln!("skipping recon_dav1d: dav1d not installed");
        return;
    }
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
fn flat_lossy_reconstruction_matches_dav1d() {
    if !dav1d_available() {
        return;
    }
    // A solid color: every residual quantizes to zero, so the reconstruction is the DC prediction
    // chain — a clean test that prediction-from-reconstruction tracks the decoder exactly.
    check(&planes(48, 40, |_, _| [200, 100, 50]), 16);
}
