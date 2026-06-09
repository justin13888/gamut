//! The deblocking loop filter (§7.14), applied to the reconstructed frame after all blocks are
//! decoded. It reduces the blocking artifacts introduced by independent per-block coding.
//!
//! This implements the subset the lossy still-image path needs: all blocks are 4×4 under
//! `TX_MODE_LARGEST`, so every transform/prediction edge coincides and `filterSize == 4` everywhere
//! — only the narrow (4-tap) filter (§7.14.6.3) runs. The frame is a single intra key-frame with one
//! segment, `loop_filter_sharpness = 0`, and `loop_filter_delta_enabled = 0`, so the filter strength
//! is a single signaled `loop_filter_level` (the same value for both luma passes and both chroma
//! planes). The filter is a pure post-process: intra prediction during encoding reads the
//! *pre-filter* reconstruction (§7.4 applies the loop filter only after the tile is decoded), so the
//! encoder applies this once to its final reconstruction to match a conformant decoder's output.

/// The `loop_filter_level` (§5.9.11) the encoder signals and applies as a function of `base_q_idx`.
/// `0` disables the filter (the level is then omitted for chroma and no filtering occurs). The exact
/// level is a quality decision (rate-distortion selection is deferred); this is a deterministic
/// monotonic placeholder that scales filtering with the quantizer.
pub(crate) fn deblock_level(qindex: u8) -> u8 {
    (u32::from(qindex) / 4).min(63) as u8
}

/// `Round2Signed`-free narrow (4-tap) deblock of one boundary sample (§7.14.6.2 mask + §7.14.6.3
/// filter, 8-bit). `base` is the index of `q0` (the first sample on the near side of the edge);
/// `step` is the index stride between successive samples perpendicular to the edge (`1` for a
/// vertical edge, the row stride for a horizontal edge). `p0`/`p1` lie at `base - step` /
/// `base - 2*step`, `q1` at `base + step`.
fn narrow_filter(buf: &mut [u8], base: usize, step: usize, limit: i32, blimit: i32, thresh: i32) {
    let q0 = i32::from(buf[base]);
    let q1 = i32::from(buf[base + step]);
    let p0 = i32::from(buf[base - step]);
    let p1 = i32::from(buf[base - 2 * step]);

    // filterMask (§7.14.6.2): bail unless the samples straddling the edge are close enough.
    let mask = (p1 - p0).abs() > limit
        || (q1 - q0).abs() > limit
        || (p0 - q0).abs() * 2 + (p1 - q1).abs() / 2 > blimit;
    if mask {
        return;
    }
    // hevMask: high edge variance ⇒ only the innermost sample on each side is modified.
    let hev = (p1 - p0).abs() > thresh || (q1 - q0).abs() > thresh;

    let clamp = |v: i32| v.clamp(-128, 127);
    let (ps1, ps0, qs0, qs1) = (p1 - 128, p0 - 128, q0 - 128, q1 - 128);
    let mut filter = if hev { clamp(ps1 - qs1) } else { 0 };
    filter = clamp(filter + 3 * (qs0 - ps0));
    let filter1 = clamp(filter + 4) >> 3;
    let filter2 = clamp(filter + 3) >> 3;
    buf[base] = (clamp(qs0 - filter1) + 128) as u8; // oq0
    buf[base - step] = (clamp(ps0 + filter2) + 128) as u8; // op0
    if !hev {
        let f = (filter1 + 1) >> 1; // Round2(filter1, 1)
        buf[base + step] = (clamp(qs1 - f) + 128) as u8; // oq1
        buf[base - 2 * step] = (clamp(ps1 + f) + 128) as u8; // op1
    }
}

/// Applies the deblocking loop filter to the coded-grid reconstruction planes in place (§7.14).
/// `coded_w` is the plane row stride; `width`/`height` are the visible (display) dimensions for the
/// `onScreen` test. The filter follows the spec's 4×4 MI-grid iteration: an edge whose origin lies
/// inside the visible area is filtered across all four samples of the MI block, which can extend into
/// the coded-grid padding at a partial bottom/right edge — exactly as a conformant decoder does (the
/// padding is then cropped away; the coded grid is a multiple of 8, so these reads stay in bounds).
/// All planes are 4:4:4, so the chroma planes share the luma grid.
pub(crate) fn deblock(
    planes: &mut [Vec<u8>; 3],
    coded_w: usize,
    width: usize,
    height: usize,
    qindex: u8,
) {
    let lvl = i32::from(deblock_level(qindex));
    if lvl == 0 {
        return;
    }
    // sharpness = 0 ⇒ shift = 0; the same level applies to both luma passes and both chroma planes.
    let limit = lvl.max(1);
    let blimit = 2 * (lvl + 2) + limit;
    let thresh = lvl >> 4;

    for plane in planes.iter_mut() {
        // Pass 0: vertical edges. Each on-screen edge (column x = 4, 8, … inside the frame) filters
        // the four sample rows of its MI block. The whole pass must finish before the horizontal
        // pass, which reads the vertical-filtered samples.
        let mut x = 4;
        while x < width {
            let mut y = 0;
            while y < height {
                for i in 0..4 {
                    narrow_filter(plane, (y + i) * coded_w + x, 1, limit, blimit, thresh);
                }
                y += 4;
            }
            x += 4;
        }
        // Pass 1: horizontal edges (row y = 4, 8, … inside the frame), filtering the four columns of
        // each MI block.
        let mut y = 4;
        while y < height {
            let mut x = 0;
            while x < width {
                for i in 0..4 {
                    narrow_filter(plane, y * coded_w + (x + i), coded_w, limit, blimit, thresh);
                }
                x += 4;
            }
            y += 4;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_is_monotonic_and_clamped() {
        assert_eq!(deblock_level(0), 0);
        assert_eq!(deblock_level(4), 1);
        assert_eq!(deblock_level(20), 5);
        assert_eq!(deblock_level(255), 63); // 255/4 = 63
        for q in 0u8..=254 {
            assert!(deblock_level(q) <= deblock_level(q + 1));
        }
    }

    #[test]
    fn narrow_filter_smooths_a_small_step() {
        // p1,p0,q0,q1 = 100,100,108,108 (a step of 8). At lvl = 10 (limit 10, blimit 34, thresh 0)
        // the filterMask passes and, with no high edge variance, two samples each side are adjusted
        // toward a gradient. Hand-traced: filter = 24, filter1 = filter2 = 3, f = 2.
        let mut buf = [100u8, 100, 108, 108];
        narrow_filter(&mut buf, 2, 1, 10, 34, 0);
        assert_eq!(buf, [102, 103, 105, 106]);
    }

    #[test]
    fn narrow_filter_skips_large_step() {
        // A step of 20 exceeds blimit at lvl 10, so filterMask is 0 and nothing changes.
        let mut buf = [100u8, 100, 120, 120];
        narrow_filter(&mut buf, 2, 1, 10, 34, 0);
        assert_eq!(buf, [100, 100, 120, 120]);
    }

    #[test]
    fn deblock_leaves_flat_image_unchanged_but_filters_edges() {
        // Flat plane: every difference is zero, so the filter is a no-op.
        let mut flat = [
            vec![128u8; 16 * 16],
            vec![128u8; 16 * 16],
            vec![128u8; 16 * 16],
        ];
        deblock(&mut flat, 16, 16, 16, 64);
        assert!(flat[0].iter().all(|&v| v == 128));

        // A vertical step at column 8 gets smoothed across the x = 8 boundary.
        let mut planes = [vec![0u8; 16 * 16], vec![0u8; 16 * 16], vec![0u8; 16 * 16]];
        for y in 0..16 {
            for x in 0..16 {
                planes[0][y * 16 + x] = if x < 8 { 100 } else { 108 };
            }
        }
        let before = planes[0].clone();
        deblock(&mut planes, 16, 16, 16, 64); // qindex 64 ⇒ level 16
        assert_ne!(
            planes[0], before,
            "deblock should modify samples near the edge"
        );
        // Samples far from any 4-boundary are untouched; the edge column pair is pulled together.
        assert!(planes[0][8] < 108 && planes[0][7] > 100);
    }
}
