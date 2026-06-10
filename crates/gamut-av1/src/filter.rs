//! The in-loop filters applied to the reconstructed frame after all blocks are decoded: the
//! **deblocking loop filter** (§7.14, applied first) then **CDEF** (§7.15, deringing). They run in
//! the decoder's order (deblock → CDEF) and are pure post-processes — intra prediction during
//! encoding reads the *pre-filter* reconstruction (§7.4 applies the loop filters only after the tile
//! is decoded), so the encoder applies them once to its final reconstruction to match a conformant
//! decoder's output.
//!
//! Deblock: under `TX_MODE_SELECT` a luma block (≤32×32) uses one block-size or several smaller
//! square transforms; 4:4:4 chroma always uses the block-size transform. `filterSize` is
//! `Min(16 luma / 8 chroma, baseSize)` where `baseSize` is the
//! minimum transform width across the edge: 4 (narrow, §7.14.6.3) for any edge touching a 4×4
//! transform, 8 (wide, §7.14.6.4 `log2Size = 3`) for an 8↔8 edge, and 16 (wide, §7.14.6.4
//! `log2Size = 4`) for a 16↔16 luma edge. The frame is a single intra key-frame with one segment,
//! `loop_filter_sharpness = 0`, and `loop_filter_delta_enabled = 0`; `delta_lf_present = 1`, so the
//! level is the frame `loop_filter_level` plus a per-superblock `DeltaLF` (each edge takes the level
//! of its q0-side block, §7.14.4 — the same `loop_filter_level` for both luma passes and chroma).
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

/// The per-edge loop-filter strength thresholds (§7.14.4), all derived from `loop_filter_level` at
/// `loop_filter_sharpness = 0`: `limit`/`blimit` gate whether filtering happens, `thresh` the
/// high-edge-variance test.
#[derive(Clone, Copy)]
struct Strength {
    limit: i32,
    blimit: i32,
    thresh: i32,
}

/// Computes the deblock masks (§7.14.6.2) for one boundary sample and, if the filter applies,
/// returns `(hevMask, flatMask)`. `base` is the index of `q0`; `step` the stride perpendicular to
/// the edge (`1` vertical, row stride horizontal). `filter_size` is 4 or 8; `is_luma` selects the
/// tap count. Returns `None` when `filterMask == 0` (no filtering). 8-bit (`BitDepth == 8`).
fn deblock_masks(
    buf: &[u8],
    base: usize,
    step: usize,
    filter_size: usize,
    is_luma: bool,
    st: Strength,
) -> Option<(bool, bool, bool)> {
    let Strength {
        limit,
        blimit,
        thresh,
    } = st;
    let s = |k: isize| -> i32 { i32::from(buf[(base as isize + k * step as isize) as usize]) };
    let (q0, q1, p0, p1) = (s(0), s(1), s(-1), s(-2));

    // filterLen: 4 for the narrow case; chroma wide = 6; luma wide (filterSize 8) = 8. Samples
    // beyond ±2 are only read (and only in bounds) when the corresponding filterLen requires them.
    let filter_len = if filter_size == 4 {
        4
    } else if !is_luma {
        6
    } else {
        8
    };

    let hev = (p1 - p0).abs() > thresh || (q1 - q0).abs() > thresh;

    let mut mask = (p1 - p0).abs() > limit
        || (q1 - q0).abs() > limit
        || (p0 - q0).abs() * 2 + (p1 - q1).abs() / 2 > blimit;
    let (q2, p2) = if filter_len >= 6 {
        let (q2, p2) = (s(2), s(-3));
        mask |= (p2 - p1).abs() > limit || (q2 - q1).abs() > limit;
        (q2, p2)
    } else {
        (0, 0)
    };
    let (q3, p3) = if filter_len >= 8 {
        let (q3, p3) = (s(3), s(-4));
        mask |= (p3 - p2).abs() > limit || (q3 - q2).abs() > limit;
        (q3, p3)
    } else {
        (0, 0)
    };
    if mask {
        return None; // filterMask == 0
    }

    // flatMask (only meaningful for filterSize >= 8, where filterLen >= 6 so p2/q2 are read);
    // thresholdBd = 1 << (BitDepth - 8) = 1.
    let flat = if filter_size >= 8 {
        let t = 1;
        let mut fm = (p1 - p0).abs() > t
            || (q1 - q0).abs() > t
            || (p2 - p0).abs() > t
            || (q2 - q0).abs() > t;
        if filter_len >= 8 {
            fm |= (p3 - p0).abs() > t || (q3 - q0).abs() > t;
        }
        !fm
    } else {
        false
    };

    // flatMask2 (only required for filterSize == 16): are the outer samples p4..p6 / q4..q6 also flat?
    // Only read at the wide 16-tap luma edges, where p6 = s(-7) / q6 = s(6) are guaranteed in bounds.
    let flat2 = if filter_size >= 16 {
        let t = 1;
        let (q4, q5, q6, p4, p5, p6) = (s(4), s(5), s(6), s(-5), s(-6), s(-7));
        let fm = (p6 - p0).abs() > t
            || (q6 - q0).abs() > t
            || (p5 - p0).abs() > t
            || (q5 - q0).abs() > t
            || (p4 - p0).abs() > t
            || (q4 - q0).abs() > t;
        !fm
    } else {
        false
    };
    Some((hev, flat, flat2))
}

/// Narrow (4-tap) deblock filter (§7.14.6.3): modifies up to two samples each side of the edge.
fn narrow_apply(buf: &mut [u8], base: usize, step: usize, hev: bool) {
    let q0 = i32::from(buf[base]);
    let q1 = i32::from(buf[base + step]);
    let p0 = i32::from(buf[base - step]);
    let p1 = i32::from(buf[base - 2 * step]);
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

/// Wide (low-pass) deblock filter (§7.14.6.4), parameterized by `log2_size`. `log2_size == 4` (a
/// 16↔16 luma edge) uses `n = 6, n2 = 1` (modifies 6 samples each side from p6..q6); `log2_size == 3`
/// uses `n = 3, n2 = 0` for luma (3 each side) and `n = 2, n2 = 1` for chroma (2 each side). All taps
/// read the original samples, then write.
fn wide_apply(buf: &mut [u8], base: usize, step: usize, is_luma: bool, log2_size: u32) {
    let (n, n2): (isize, isize) = if log2_size == 4 {
        (6, 1)
    } else if is_luma {
        (3, 0)
    } else {
        (2, 1)
    };
    let read = |k: isize| -> i32 { i32::from(buf[(base as isize + k * step as isize) as usize]) };
    let mut out = [0i32; 12]; // up to 2*n = 12 modified samples (i = -n .. n-1)
    let rnd = 1i32 << (log2_size - 1);
    for i in -n..n {
        let mut t = 0i32;
        for j in -n..=n {
            let p = (i + j).clamp(-(n + 1), n);
            let tap = if j.abs() <= n2 { 2 } else { 1 };
            t += read(p) * tap;
        }
        out[(i + n) as usize] = (t + rnd) >> log2_size; // Round2(t, log2Size)
    }
    for i in -n..n {
        buf[(base as isize + i * step as isize) as usize] = out[(i + n) as usize] as u8;
    }
}

/// Filters one boundary sample: computes the masks, then applies the filter per the §7.14.6.1
/// selection — narrow if `filterSize == 4` or `!flatMask`; else wide `log2Size == 3` if
/// `filterSize == 8` or `!flatMask2`; else (a 16↔16 luma edge) wide `log2Size == 4`.
fn filter_sample(
    buf: &mut [u8],
    base: usize,
    step: usize,
    filter_size: usize,
    is_luma: bool,
    st: Strength,
) {
    if let Some((hev, flat, flat2)) = deblock_masks(buf, base, step, filter_size, is_luma, st) {
        if filter_size == 4 || !flat {
            narrow_apply(buf, base, step, hev);
        } else if filter_size == 8 || !flat2 {
            wide_apply(buf, base, step, is_luma, 3);
        } else {
            wide_apply(buf, base, step, is_luma, 4);
        }
    }
}

/// Applies the deblocking loop filter to the coded-grid reconstruction planes in place (§7.14).
/// `coded_w` is the plane row stride; `mi_cols` strides the per-MI maps; `width`/`height` are the
/// visible dimensions for the `onScreen` test. The luma transform size per 4×4 MI cell comes from
/// `tx_log2` (which, under `TX_MODE_SELECT`, can be smaller than the block); 4:4:4 chroma always uses
/// the block-size transform, so its size is `mi_bsl + 2`. Only transform-block edges are filtered; the
/// filter size is `Min(Tx_Width)` across the edge (capped at 16 luma / 8 chroma) — 4 (narrow) for any
/// edge touching a 4×4 transform, 8 (wide) for an 8↔8 edge, 16 (wide) for a 16↔16 luma edge. Partial
/// bottom/right edges filter into the coded-grid padding exactly as a conformant decoder does (the
/// coded grid is a multiple of 8, so the reads stay in bounds); the padding is then cropped.
#[allow(clippy::too_many_arguments)]
pub(crate) fn deblock(
    planes: &mut [Vec<u8>; 3],
    coded_w: usize,
    mi_cols: usize,
    width: usize,
    height: usize,
    tx_log2: &[u8],
    tx_log2_h: &[u8],
    mi_bsl: &[u8],
    mi_bsl_h: &[u8],
    mi_dlf: &[i8],
    qindex: u8,
) {
    let base_lvl = i32::from(deblock_level(qindex));
    if base_lvl == 0 {
        return; // frame loop_filter_level 0 ⇒ delta_lf is also 0 (encoder), so every level is 0.
    }
    // The per-superblock loop-filter level is `Clip3(0, 63, loop_filter_level + DeltaLF)`; with
    // `loop_filter_sharpness = 0` the strength thresholds derive from it directly (§7.14.4/.5).
    let strength_for = |cell: usize| -> Strength {
        let lvl = (base_lvl + i32::from(mi_dlf[cell])).clamp(0, 63);
        let limit = lvl.max(1);
        Strength {
            limit,
            blimit: 2 * (lvl + 2) + limit,
            thresh: lvl >> 4,
        }
    };

    for (plane_idx, plane) in planes.iter_mut().enumerate() {
        let is_luma = plane_idx == 0;
        let size_cap = if is_luma { 16 } else { 8 };
        // Transform width/height (log2) for this plane at MI `cell`: luma uses the signaled tx size;
        // 4:4:4 chroma uses the block-size transform (`mi_bsl + 2`) capped at TX_32X32 (chroma never
        // uses TX_64X64, so a 64×64 block's chroma is a raster of 32×32 transforms with an edge at 32).
        let txlog2 = |cell: usize| -> u32 {
            if is_luma {
                u32::from(tx_log2[cell])
            } else {
                (u32::from(mi_bsl[cell]) + 2).min(5)
            }
        };
        // The transform *height* (log2) — equals `txlog2` for square transforms, differs for
        // rectangular ones; the horizontal pass sizes its edges by this.
        let txlog2_h = |cell: usize| -> u32 {
            if is_luma {
                u32::from(tx_log2_h[cell])
            } else {
                (u32::from(mi_bsl_h[cell]) + 2).min(5)
            }
        };

        // Pass 0: vertical edges. Each on-screen edge (column x = 4, 8, … inside the frame) filters
        // the four sample rows of its MI block, when the column is a transform-block edge. The whole
        // pass must finish before the horizontal pass, which reads the vertical-filtered samples.
        let mut x = 4;
        while x < width {
            let col = x >> 2;
            let mut y = 0;
            while y < height {
                let row = y >> 2;
                let txw = 1usize << txlog2(row * mi_cols + col);
                if x % txw == 0 {
                    let prev_txw = 1usize << txlog2(row * mi_cols + (col - 1));
                    let filter_size = prev_txw.min(txw).min(size_cap);
                    // The edge takes the level of its q0-side (right) block (§7.14.4).
                    let st = strength_for(row * mi_cols + col);
                    for i in 0..4 {
                        filter_sample(plane, (y + i) * coded_w + x, 1, filter_size, is_luma, st);
                    }
                }
                y += 4;
            }
            x += 4;
        }
        // Pass 1: horizontal edges (row y = 4, 8, … inside the frame), filtering the four columns of
        // each MI block when the row is a transform-block edge.
        let mut y = 4;
        while y < height {
            let row = y >> 2;
            let mut x = 0;
            while x < width {
                let col = x >> 2;
                let txh = 1usize << txlog2_h(row * mi_cols + col);
                if y % txh == 0 {
                    let prev_txh = 1usize << txlog2_h((row - 1) * mi_cols + col);
                    let filter_size = prev_txh.min(txh).min(size_cap);
                    // The edge takes the level of its q0-side (bottom) block (§7.14.4).
                    let st = strength_for(row * mi_cols + col);
                    for i in 0..4 {
                        filter_sample(
                            plane,
                            y * coded_w + (x + i),
                            coded_w,
                            filter_size,
                            is_luma,
                            st,
                        );
                    }
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
pub(crate) fn cdef(
    planes: &[Vec<u8>; 3],
    coded_w: usize,
    mi_skip: &[u8],
    mi_cols: usize,
    qindex: u8,
) -> [Vec<u8>; 3] {
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
            // §7.15.1: CDEF is not applied to an 8×8 block whose four 4×4 cells are all `skip`.
            let (r4, c4) = (y0 / 4, x0 / 4);
            let all_skip = mi_skip[r4 * mi_cols + c4] != 0
                && mi_skip[r4 * mi_cols + (c4 + 1)] != 0
                && mi_skip[(r4 + 1) * mi_cols + c4] != 0
                && mi_skip[(r4 + 1) * mi_cols + (c4 + 1)] != 0;
            if all_skip {
                x0 += 8;
                continue;
            }
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
        filter_sample(
            &mut buf,
            2,
            1,
            4,
            true,
            Strength {
                limit: 10,
                blimit: 34,
                thresh: 0,
            },
        );
        assert_eq!(buf, [102, 103, 105, 106]);
    }

    #[test]
    fn narrow_filter_skips_large_step() {
        // A step of 20 exceeds blimit at lvl 10, so filterMask is 0 and nothing changes.
        let mut buf = [100u8, 100, 120, 120];
        filter_sample(
            &mut buf,
            2,
            1,
            4,
            true,
            Strength {
                limit: 10,
                blimit: 34,
                thresh: 0,
            },
        );
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
        // 16×16 ⇒ 4×4 MI grid; no block is skip, so CDEF visits every 8×8.
        let mi_skip = vec![0u8; 4 * 4];
        assert_eq!(cdef(&flat, 16, &mi_skip, 4, 255), flat);

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
        let out = cdef(&planes, 16, &mi_skip, 4, 255);
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
        // 16×16 ⇒ 4×4 MI grid, all 4×4 transforms (tx_log2 = 2 everywhere ⇒ narrow filter). The
        // 4×4 blocks have `mi_bsl = 0`, so chroma tx is also 4×4 (log2 = 0 + 2).
        let tx_log2 = vec![2u8; 4 * 4];
        let mi_bsl = vec![0u8; 4 * 4];
        let mi_dlf = vec![0i8; 4 * 4];
        deblock(
            &mut flat, 16, 4, 16, 16, &tx_log2, &tx_log2, &mi_bsl, &mi_bsl, &mi_dlf, 64,
        );
        assert!(flat[0].iter().all(|&v| v == 128));

        // A vertical step at column 8 gets smoothed across the x = 8 boundary.
        let mut planes = [vec![0u8; 16 * 16], vec![0u8; 16 * 16], vec![0u8; 16 * 16]];
        for y in 0..16 {
            for x in 0..16 {
                planes[0][y * 16 + x] = if x < 8 { 100 } else { 108 };
            }
        }
        let before = planes[0].clone();
        deblock(
            &mut planes,
            16,
            4,
            16,
            16,
            &tx_log2,
            &tx_log2,
            &mi_bsl,
            &mi_bsl,
            &mi_dlf,
            64,
        ); // qindex 64 ⇒ level 16
        assert_ne!(
            planes[0], before,
            "deblock should modify samples near the edge"
        );
        // Samples far from any 4-boundary are untouched; the edge column pair is pulled together.
        assert!(planes[0][8] < 108 && planes[0][7] > 100);
    }
}
