//! PNG scanline filtering (PNG spec §9).
//!
//! Before compression, each scanline is transformed by one of five filters that predict each byte
//! from its neighbours (left, above, above-left) and store the residual. Good filter choices make
//! the data far more compressible. Filters operate on raw bytes with a stride of `bpp` (bytes per
//! pixel, ≥1); they reference the *unfiltered* bytes of the current and previous rows.

/// A PNG scanline filter type (the leading byte of each filtered scanline).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FilterType {
    /// No filtering; the bytes are stored as-is.
    None = 0,
    /// Residual from the byte `bpp` to the left.
    Sub = 1,
    /// Residual from the byte directly above.
    Up = 2,
    /// Residual from the floor-average of the left and above bytes.
    Average = 3,
    /// Residual from the Paeth predictor of left, above, and above-left.
    Paeth = 4,
}

/// How the encoder chooses a filter for each scanline (a space/time trade-off).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterStrategy {
    /// Filter every scanline with [`FilterType::None`] (fastest; good for already-random data).
    None,
    /// Use one fixed filter for every scanline.
    Fixed(FilterType),
    /// Per scanline, pick the filter minimising the sum of absolute residuals — the standard
    /// libpng heuristic. A good size/speed balance and the default.
    MinSumAbs,
    /// Encode the whole image under several filter strategies, DEFLATE each, and keep the smallest.
    /// Pairs with [`Level::Best`](gamut_deflate::Level::Best) for maximum compression; slowest.
    BruteForce,
}

impl FilterType {
    /// The filter type for a scanline's leading filter byte, or `None` for an undefined code
    /// (PNG §9.1 defines exactly 0–4 for filter method 0).
    pub(crate) fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(FilterType::None),
            1 => Some(FilterType::Sub),
            2 => Some(FilterType::Up),
            3 => Some(FilterType::Average),
            4 => Some(FilterType::Paeth),
            _ => None,
        }
    }
}

/// The Paeth predictor (PNG §9.4): chooses whichever of `a` (left), `b` (above), `c` (above-left)
/// is closest to `a + b - c`, with the spec's exact tie-break order.
fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (ai, bi, ci) = (i32::from(a), i32::from(b), i32::from(c));
    let p = ai + bi - ci;
    let pa = (p - ai).abs();
    let pb = (p - bi).abs();
    let pc = (p - ci).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Forward-filters one scanline `cur` (with previous raw row `prev`, all zero for the first row)
/// into `out` (which is overwritten to `cur.len()` bytes).
fn filter_row(filter: FilterType, cur: &[u8], prev: &[u8], bpp: usize, out: &mut Vec<u8>) {
    out.clear();
    out.reserve(cur.len());
    for i in 0..cur.len() {
        let a = if i >= bpp { cur[i - bpp] } else { 0 };
        let b = prev[i];
        let c = if i >= bpp { prev[i - bpp] } else { 0 };
        let residual = match filter {
            FilterType::None => cur[i],
            FilterType::Sub => cur[i].wrapping_sub(a),
            FilterType::Up => cur[i].wrapping_sub(b),
            FilterType::Average => cur[i].wrapping_sub(((u16::from(a) + u16::from(b)) / 2) as u8),
            FilterType::Paeth => cur[i].wrapping_sub(paeth(a, b, c)),
        };
        out.push(residual);
    }
}

/// Reconstructs one scanline in place from its filtered bytes (PNG §9.2–§9.4, the decoder's
/// inverse of [`filter_row`]): `row` holds the filtered bytes on entry and the raw bytes on exit.
/// `prev` is the *raw* previous scanline of the same (reduced) image — all zero for the first row —
/// and `bpp` is the filter stride in bytes (≥ 1). In-place works because reconstruction consumes
/// left-to-right: `row[i - bpp]` is already raw when `row[i]` needs it.
pub(crate) fn unfilter_row(filter: FilterType, row: &mut [u8], prev: &[u8], bpp: usize) {
    for i in 0..row.len() {
        let a = if i >= bpp { row[i - bpp] } else { 0 };
        let b = prev[i];
        let c = if i >= bpp { prev[i - bpp] } else { 0 };
        row[i] = match filter {
            FilterType::None => row[i],
            FilterType::Sub => row[i].wrapping_add(a),
            FilterType::Up => row[i].wrapping_add(b),
            FilterType::Average => row[i].wrapping_add(((u16::from(a) + u16::from(b)) / 2) as u8),
            FilterType::Paeth => row[i].wrapping_add(paeth(a, b, c)),
        };
    }
}

/// The minimum-sum-of-absolute-residuals score for a filtered row (each byte counted as a signed
/// magnitude). Lower is more compressible.
fn sum_abs(filtered: &[u8]) -> u64 {
    filtered
        .iter()
        .map(|&x| u64::from(x.min(x.wrapping_neg())))
        .sum()
}

/// Filters every scanline of `samples` (row-major, `row_bytes` per row) per `strategy`, producing
/// the filter-prefixed byte stream that gets compressed: a filter-type byte then the filtered row,
/// for each scanline. `bpp` is the filter stride (bytes per pixel, ≥1).
pub fn filter_image(
    strategy: FilterStrategy,
    samples: &[u8],
    row_bytes: usize,
    bpp: usize,
) -> Vec<u8> {
    let height = samples.len().checked_div(row_bytes).unwrap_or(0);
    let mut out = Vec::with_capacity((row_bytes + 1) * height);
    let zero_row = vec![0u8; row_bytes];
    let mut prev = zero_row.as_slice();
    let mut scratch = Vec::with_capacity(row_bytes);
    for y in 0..height {
        let cur = &samples[y * row_bytes..(y + 1) * row_bytes];
        let filter = match strategy {
            FilterStrategy::None => FilterType::None,
            FilterStrategy::Fixed(f) => f,
            // BruteForce is resolved to concrete strategies by the encoder; if it reaches here, fall
            // back to the per-scanline heuristic.
            FilterStrategy::MinSumAbs | FilterStrategy::BruteForce => {
                choose_min_sum_abs(cur, prev, bpp, &mut scratch)
            }
        };
        out.push(filter as u8);
        filter_row(filter, cur, prev, bpp, &mut scratch);
        out.extend_from_slice(&scratch);
        prev = cur;
    }
    out
}

/// Picks the filter with the lowest sum-of-absolute-residuals for one scanline.
pub fn choose_min_sum_abs(
    cur: &[u8],
    prev: &[u8],
    bpp: usize,
    scratch: &mut Vec<u8>,
) -> FilterType {
    let mut best = FilterType::None;
    let mut best_score = u64::MAX;
    for filter in [
        FilterType::None,
        FilterType::Sub,
        FilterType::Up,
        FilterType::Average,
        FilterType::Paeth,
    ] {
        filter_row(filter, cur, prev, bpp, scratch);
        let score = sum_abs(scratch);
        if score < best_score {
            best_score = score;
            best = filter;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reconstructs a scanline from its filtered form via the production [`unfilter_row`].
    fn reconstruct(filter: FilterType, filt: &[u8], prev: &[u8], bpp: usize) -> Vec<u8> {
        let mut cur = filt.to_vec();
        unfilter_row(filter, &mut cur, prev, bpp);
        cur
    }

    #[test]
    fn every_filter_is_invertible() {
        let filters = [
            FilterType::None,
            FilterType::Sub,
            FilterType::Up,
            FilterType::Average,
            FilterType::Paeth,
        ];
        for bpp in [1usize, 2, 3, 4] {
            for seed in 0..8u32 {
                let n = 4 * bpp + (seed as usize % 5);
                let cur: Vec<u8> = (0..n)
                    .map(|i| {
                        (i as u32)
                            .wrapping_mul(seed.wrapping_add(7))
                            .wrapping_mul(31) as u8
                    })
                    .collect();
                let prev: Vec<u8> = (0..n)
                    .map(|i| (i as u32 ^ seed.wrapping_mul(13)).wrapping_mul(17) as u8)
                    .collect();
                for &f in &filters {
                    let mut filt = Vec::new();
                    filter_row(f, &cur, &prev, bpp, &mut filt);
                    assert_eq!(reconstruct(f, &filt, &prev, bpp), cur, "{f:?} bpp={bpp}");
                }
            }
        }
    }

    #[test]
    fn paeth_matches_spec_examples() {
        // a + b - c closest wins; ties favour a, then b.
        assert_eq!(paeth(10, 20, 10), 20); // p=20 -> b
        assert_eq!(paeth(100, 90, 80), 100); // p=110 -> a (|10| vs |20| vs |30|)
        assert_eq!(paeth(0, 0, 0), 0);
        assert_eq!(paeth(255, 0, 0), 255); // p=255 -> a
    }

    #[test]
    fn paeth_tie_breaks_exactly_per_spec() {
        // The observable tie is pb == pc < pa, where the spec's order picks b over c: with
        // c = (2a + b) / 3, pb = pc = |a-b|/3 while pa = 2|a-b|/3.
        assert_eq!(paeth(0, 9, 3), 9); // pa=6, pb=pc=3 -> b (a "<" mutant would pick c)
        assert_eq!(paeth(90, 0, 60), 0); // pa=60, pb=pc=30 -> b
        // pa == pb with distinct a and b forces pc = 0, so c wins outright.
        assert_eq!(paeth(10, 30, 20), 20); // p=20: pa=pb=10, pc=0 -> c
        assert_eq!(paeth(100, 60, 80), 80); // p=80: pa=pb=20, pc=0 -> c
    }

    #[test]
    fn unfilter_golden_vectors() {
        // Hand-computed reconstructions with non-trivial predecessors (bpp = 2).
        let prev = [10u8, 20, 30, 40, 50, 60];
        let mut sub = [1u8, 2, 3, 4, 5, 6];
        unfilter_row(FilterType::Sub, &mut sub, &prev, 2);
        assert_eq!(sub, [1, 2, 4, 6, 9, 12]);
        let mut up = [1u8, 2, 3, 4, 5, 6];
        unfilter_row(FilterType::Up, &mut up, &prev, 2);
        assert_eq!(up, [11, 22, 33, 44, 55, 66]);
        // Average: floor((a + b) / 2) with u16 widening; first pixel has a = 0.
        let mut avg = [1u8, 2, 3, 4, 5, 6];
        unfilter_row(FilterType::Average, &mut avg, &prev, 2);
        // x0: 1+10/2=6, 2+20/2=12; x1: 3+(6+30)/2=21, 4+(12+40)/2=30; x2: 5+(21+50)/2=40, 6+(30+60)/2=51
        assert_eq!(avg, [6, 12, 21, 30, 40, 51]);
        // Average overflow: a + b = 255 + 255 must widen, not wrap, before halving.
        let mut wide = [0u8, 0];
        unfilter_row(FilterType::Average, &mut wide, &[255, 255], 2);
        let mut wide2 = [1u8, 255];
        unfilter_row(FilterType::Average, &mut wide2, &[255, 255], 1);
        assert_eq!(wide, [127, 127]); // floor(255/2) twice (a = 0 for the first pixel)
        assert_eq!(wide2, [128, 190]); // 1+floor(255/2)=128, then 255+floor((128+255)/2)=255+191 wraps to 190
    }

    #[test]
    fn min_sum_abs_prefers_flat_residuals() {
        // A horizontal gradient (each pixel = previous + k) filters to a constant under Sub, which
        // scores far below None.
        let row: Vec<u8> = (0..30u8).map(|i| i.wrapping_mul(3)).collect();
        let prev = vec![0u8; row.len()];
        let chosen = choose_min_sum_abs(&row, &prev, 1, &mut Vec::new());
        assert_eq!(chosen, FilterType::Sub);
    }
}
