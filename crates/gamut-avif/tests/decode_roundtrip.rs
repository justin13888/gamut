//! End-to-end correctness: encode → `avifdec` → Y4M → compare planes to the source.
//!
//! This is the authoritative correctness check (a real AV1 decoder must reproduce the input
//! bit-exactly under lossless coding). It is skipped when `avifdec` (libavif) is not installed, so
//! CI without the tool still passes; the hermetic unit tests carry the coverage gate.

use gamut_avif::AvifEncoder;
use gamut_core::{Dimensions, Encoder};
use std::process::Command;

fn avifdec_available() -> bool {
    Command::new("avifdec")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Source RGB pattern (structure + variation to exercise nonzero coefficients).
fn rgb_at(x: u32, y: u32) -> (u8, u8, u8) {
    (
        ((x * 7 + y * 3) & 0xff) as u8,
        ((x * x + y) & 0xff) as u8,
        ((x ^ (y * 5)) & 0xff) as u8,
    )
}

fn roundtrip(w: u32, h: u32) {
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

    let mut avif = Vec::new();
    AvifEncoder::new()
        .encode(
            &rgb,
            Dimensions {
                width: w,
                height: h,
            },
            &mut avif,
        )
        .unwrap();

    let dir = std::env::temp_dir();
    let base = format!("gamut_rt_{}_{w}x{h}", std::process::id());
    let avif_path = dir.join(format!("{base}.avif"));
    let y4m_path = dir.join(format!("{base}.y4m"));
    std::fs::write(&avif_path, &avif).unwrap();

    let out = Command::new("avifdec")
        .arg(&avif_path)
        .arg(&y4m_path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "avifdec failed for {w}x{h}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let y4m = std::fs::read(&y4m_path).unwrap();
    let _ = std::fs::remove_file(&avif_path);
    let _ = std::fs::remove_file(&y4m_path);

    // Y4M: "<header>\nFRAME\n" then Y, U, V planes (w*h bytes each for C444).
    let hdr_end = y4m.iter().position(|&b| b == b'\n').unwrap();
    let after = &y4m[hdr_end + 1..];
    let frame_end = after.iter().position(|&b| b == b'\n').unwrap();
    let planes = &after[frame_end + 1..];
    let n = (w * h) as usize;
    let (yp, up, vp) = (&planes[0..n], &planes[n..2 * n], &planes[2 * n..3 * n]);

    // Identity matrix mapping: Y=G, U=B, V=R.
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            let (r, g, b) = rgb_at(x, y);
            assert_eq!(yp[i], g, "Y!=G at ({x},{y}) in {w}x{h}");
            assert_eq!(up[i], b, "U!=B at ({x},{y}) in {w}x{h}");
            assert_eq!(vp[i], r, "V!=R at ({x},{y}) in {w}x{h}");
        }
    }
}

#[test]
fn lossless_roundtrip_via_avifdec() {
    if !avifdec_available() {
        eprintln!("skipping decode_roundtrip: avifdec (libavif) not installed");
        return;
    }
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
