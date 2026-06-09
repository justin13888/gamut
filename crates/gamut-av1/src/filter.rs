//! The in-loop filters applied to the reconstructed frame after all blocks are decoded: the
//! **deblocking loop filter** (§7.14, applied first) then **CDEF** (§7.15, deringing). They run in
//! the decoder's order (deblock → CDEF) and are pure post-processes — intra prediction during
//! encoding reads the *pre-filter* reconstruction (§7.4 applies the loop filters only after the tile
//! is decoded), so the encoder applies them once to its final reconstruction to match a conformant
//! decoder's output.
//!
//! Deblock: all blocks are 4×4 under `TX_MODE_LARGEST`, so every transform/prediction edge coincides
//! and `filterSize == 4` everywhere — only the narrow (4-tap) filter (§7.14.6.3) runs. The frame is a
//! single intra key-frame with one segment, `loop_filter_sharpness = 0`, and
//! `loop_filter_delta_enabled = 0`, so the strength is a single signaled `loop_filter_level` (the
//! same value for both luma passes and both chroma planes).
//!
//! CDEF: a single signaled strength set (`cdef_bits = 0`) is applied to every 8×8 block; the
//! per-block `cdef_idx` is `L(0)` (no tile bits). `Skips` is 0 so every block is filtered. For 4:4:4,
//! chroma shares the luma grid and `Cdef_Uv_Dir` is the identity.
//!
//! The filter strengths/levels are quality decisions (rate-distortion selection is deferred); both
//! are deterministic monotonic placeholders scaled by `base_q_idx`.

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

// ===== CDEF (Constrained Directional Enhancement Filter, §7.15) =====

/// `Cdef_Directions[8][2][2]` (§7.15.3): the (row, col) sample offsets for each of the 8 directions.
static CDEF_DIRECTIONS: [[[i32; 2]; 2]; 8] = [
    [[-1, 1], [-2, 2]],
    [[0, 1], [-1, 2]],
    [[0, 1], [0, 2]],
    [[0, 1], [1, 2]],
    [[1, 1], [2, 2]],
    [[1, 0], [2, 1]],
    [[1, 0], [2, 0]],
    [[1, 0], [2, -1]],
];
/// `Cdef_Pri_Taps[2][2]` / `Cdef_Sec_Taps[2][2]` (§7.15.3), selected by `priStr & 1`.
static CDEF_PRI_TAPS: [[i32; 2]; 2] = [[4, 2], [3, 3]];
static CDEF_SEC_TAPS: [[i32; 2]; 2] = [[2, 1], [2, 1]];
/// `Div_Table[9]` (§7.15.2).
static CDEF_DIV_TABLE: [i64; 9] = [0, 840, 420, 280, 210, 168, 140, 120, 105];

/// `FloorLog2(x)` for `x >= 1`.
fn floor_log2(x: i32) -> i32 {
    31 - (x as u32).leading_zeros() as i32
}

/// `constrain(diff, threshold, damping)` (§7.15.3): the soft-threshold applied to each neighbour
/// difference before it is weighted into the filter sum.
fn constrain(diff: i32, threshold: i32, damping: i32) -> i32 {
    if threshold == 0 {
        return 0;
    }
    let damping_adj = (damping - floor_log2(threshold)).max(0);
    let sign = if diff < 0 { -1 } else { 1 };
    sign * (threshold - (diff.abs() >> damping_adj)).clamp(0, diff.abs())
}

/// The CDEF strengths the encoder signals and applies as a function of `base_q_idx`: returns
/// `(y_pri, y_sec, uv_pri, uv_sec)` (the values stored after the §5.9.19 `3 → 4` secondary mapping).
/// All zero ⇒ CDEF is a no-op. The choice is a quality decision (RD selection is deferred); this is a
/// deterministic monotonic placeholder. `CdefDamping` is fixed at 3.
pub(crate) fn cdef_strengths(qindex: u8) -> (i32, i32, i32, i32) {
    let q = u32::from(qindex);
    let pri = (q / 20).min(15) as i32; // 0..15
    let sec = [0i32, 1, 2, 4][(q / 96).min(3) as usize]; // 0,1,2,4 (3 maps to 4)
    (pri, sec, pri, sec)
}

/// `CdefDamping` (= `cdef_damping_minus_3 + 3`); fixed at 3 here.
pub(crate) const CDEF_DAMPING: i32 = 3;

/// CDEF direction process (§7.15.2): finds the dominant direction `yDir` (0..7) and the variance
/// `var` of the luma 8×8 block whose top-left is at coded `(x0, y0)`. The index-based loops mirror
/// the spec's `partial`/`cost` accumulation directly.
#[allow(clippy::needless_range_loop)]
fn cdef_direction(luma: &[u8], coded_w: usize, x0: usize, y0: usize) -> (usize, i32) {
    let mut partial = [[0i64; 15]; 8];
    for i in 0..8 {
        for j in 0..8 {
            let x = i64::from(luma[(y0 + i) * coded_w + (x0 + j)]) - 128;
            partial[0][i + j] += x;
            partial[1][i + j / 2] += x;
            partial[2][i] += x;
            partial[3][3 + i - j / 2] += x;
            partial[4][7 + i - j] += x;
            partial[5][3 - i / 2 + j] += x;
            partial[6][j] += x;
            partial[7][i / 2 + j] += x;
        }
    }
    let mut cost = [0i64; 8];
    for i in 0..8 {
        cost[2] += partial[2][i] * partial[2][i];
        cost[6] += partial[6][i] * partial[6][i];
    }
    cost[2] *= CDEF_DIV_TABLE[8];
    cost[6] *= CDEF_DIV_TABLE[8];
    for i in 0..7 {
        cost[0] += (partial[0][i] * partial[0][i] + partial[0][14 - i] * partial[0][14 - i])
            * CDEF_DIV_TABLE[i + 1];
        cost[4] += (partial[4][i] * partial[4][i] + partial[4][14 - i] * partial[4][14 - i])
            * CDEF_DIV_TABLE[i + 1];
    }
    cost[0] += partial[0][7] * partial[0][7] * CDEF_DIV_TABLE[8];
    cost[4] += partial[4][7] * partial[4][7] * CDEF_DIV_TABLE[8];
    let mut i = 1;
    while i < 8 {
        for j in 0..5 {
            cost[i] += partial[i][3 + j] * partial[i][3 + j];
        }
        cost[i] *= CDEF_DIV_TABLE[8];
        for j in 0..3 {
            cost[i] += (partial[i][j] * partial[i][j] + partial[i][10 - j] * partial[i][10 - j])
                * CDEF_DIV_TABLE[2 * j + 2];
        }
        i += 2;
    }
    let mut best_cost = 0i64;
    let mut y_dir = 0usize;
    for (d, &c) in cost.iter().enumerate() {
        if c > best_cost {
            best_cost = c;
            y_dir = d;
        }
    }
    let var = ((best_cost - cost[(y_dir + 4) & 7]) >> 10) as i32;
    (y_dir, var)
}

/// CDEF filter process (§7.15.3) for one 8×8 (or sub-sampled) block of `plane`. Reads the
/// (deblocked) `input` and writes the filtered samples into `output`; a sample read outside the coded
/// grid is unavailable and skipped. `w`/`h` are the block extent in this plane.
#[allow(clippy::too_many_arguments)]
fn cdef_filter_block(
    input: &[u8],
    output: &mut [u8],
    coded_w: usize,
    coded_h: usize,
    x0: usize,
    y0: usize,
    w: usize,
    h: usize,
    pri_str: i32,
    sec_str: i32,
    damping: i32,
    dir: usize,
) {
    let at = |y: i32, x: i32| -> Option<i32> {
        if y >= 0 && (y as usize) < coded_h && x >= 0 && (x as usize) < coded_w {
            Some(i32::from(input[y as usize * coded_w + x as usize]))
        } else {
            None
        }
    };
    let pri_taps = &CDEF_PRI_TAPS[(pri_str & 1) as usize];
    let sec_taps = &CDEF_SEC_TAPS[(pri_str & 1) as usize];
    for i in 0..h {
        for j in 0..w {
            let x = i32::from(input[(y0 + i) * coded_w + (x0 + j)]);
            let (mut sum, mut mn, mut mx) = (0i32, x, x);
            for k in 0..2 {
                for &sign in &[-1i32, 1] {
                    let pr = at(
                        y0 as i32 + i as i32 + sign * CDEF_DIRECTIONS[dir][k][0],
                        x0 as i32 + j as i32 + sign * CDEF_DIRECTIONS[dir][k][1],
                    );
                    if let Some(p) = pr {
                        sum += pri_taps[k] * constrain(p - x, pri_str, damping);
                        mx = mx.max(p);
                        mn = mn.min(p);
                    }
                    for dir_off in [-2i32, 2] {
                        let sd = (dir as i32 + dir_off) & 7;
                        let s = at(
                            y0 as i32 + i as i32 + sign * CDEF_DIRECTIONS[sd as usize][k][0],
                            x0 as i32 + j as i32 + sign * CDEF_DIRECTIONS[sd as usize][k][1],
                        );
                        if let Some(s) = s {
                            sum += sec_taps[k] * constrain(s - x, sec_str, damping);
                            mx = mx.max(s);
                            mn = mn.min(s);
                        }
                    }
                }
            }
            let v = x + ((8 + sum - i32::from(sum < 0)) >> 4);
            output[(y0 + i) * coded_w + (x0 + j)] = v.clamp(mn, mx) as u8;
        }
    }
}

/// Applies the CDEF deringing filter (§7.15) to the deblocked reconstruction planes, returning the
/// filtered planes. Operates on the coded grid; every 8×8 luma block is filtered (`Skips` is 0 and
/// `cdef_bits` is 0, so a single signaled strength set applies everywhere). All reads are from the
/// pre-CDEF (deblocked) input, so a fresh output is produced. 4:4:4 ⇒ chroma shares the luma grid and
/// `Cdef_Uv_Dir` is the identity.
pub(crate) fn cdef(planes: &[Vec<u8>; 3], coded_w: usize, qindex: u8) -> [Vec<u8>; 3] {
    let (y_pri, y_sec, uv_pri, uv_sec) = cdef_strengths(qindex);
    let mut out = planes.clone();
    if y_pri == 0 && y_sec == 0 && uv_pri == 0 && uv_sec == 0 {
        return out;
    }
    let coded_h = planes[0].len() / coded_w;
    let mut y0 = 0;
    while y0 < coded_h {
        let mut x0 = 0;
        while x0 < coded_w {
            let (y_dir, var) = cdef_direction(&planes[0], coded_w, x0, y0);
            // Luma: the primary strength is scaled by the block variance (§7.15.1 steps 4-5).
            let dir = if y_pri == 0 { 0 } else { y_dir };
            let var_str = if var >> 6 != 0 {
                floor_log2(var >> 6).min(12)
            } else {
                0
            };
            let luma_pri = if var != 0 {
                (y_pri * (4 + var_str) + 8) >> 4
            } else {
                0
            };
            cdef_filter_block(
                &planes[0],
                &mut out[0],
                coded_w,
                coded_h,
                x0,
                y0,
                8,
                8,
                luma_pri,
                y_sec,
                CDEF_DAMPING,
                dir,
            );
            // Chroma (4:4:4): no variance scaling, damping reduced by 1, direction via the identity
            // Cdef_Uv_Dir.
            let cdir = if uv_pri == 0 { 0 } else { y_dir };
            for plane in 1..3 {
                cdef_filter_block(
                    &planes[plane],
                    &mut out[plane],
                    coded_w,
                    coded_h,
                    x0,
                    y0,
                    8,
                    8,
                    uv_pri,
                    uv_sec,
                    CDEF_DAMPING - 1,
                    cdir,
                );
            }
            x0 += 8;
        }
        y0 += 8;
    }
    out
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
    fn constrain_soft_thresholds() {
        // threshold 4, damping 3 ⇒ dampingAdj = 3 - floor_log2(4) = 1.
        assert_eq!(constrain(2, 4, 3), 2); // clamp(4 - 1, 0, 2)
        assert_eq!(constrain(-3, 4, 3), -3); // -clamp(4 - 1, 0, 3)
        assert_eq!(constrain(10, 4, 3), 0); // clamp(4 - 5, 0, 10) = 0
        assert_eq!(constrain(7, 0, 3), 0); // threshold 0 ⇒ 0
    }

    #[test]
    fn cdef_strengths_map_and_scale() {
        assert_eq!(cdef_strengths(0), (0, 0, 0, 0));
        assert_eq!(cdef_strengths(20), (1, 0, 1, 0));
        assert_eq!(cdef_strengths(96), (4, 1, 4, 1));
        assert_eq!(cdef_strengths(255), (12, 2, 12, 2));
    }

    #[test]
    fn cdef_leaves_flat_unchanged_but_filters_structure() {
        // A flat plane: every neighbour difference is zero, so constrain is zero and CDEF is identity
        // even with non-zero strengths.
        let flat = [
            vec![128u8; 16 * 16],
            vec![128u8; 16 * 16],
            vec![128u8; 16 * 16],
        ];
        assert_eq!(cdef(&flat, 16, 255), flat);

        // CDEF only attenuates *small* oscillations (large diffs are constrained away to preserve
        // edges). A low-amplitude vertical stripe gives the direction search a clear direction and a
        // small enough neighbour difference for the constrained sum to be non-zero ⇒ samples change.
        let mut planes = [
            vec![128u8; 16 * 16],
            vec![128u8; 16 * 16],
            vec![128u8; 16 * 16],
        ];
        for y in 0..16 {
            for x in 0..16 {
                planes[0][y * 16 + x] = if x % 2 == 0 { 126 } else { 132 };
            }
        }
        let out = cdef(&planes, 16, 255);
        assert_ne!(
            out[0], planes[0],
            "CDEF should dering a low-amplitude oscillation"
        );
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
