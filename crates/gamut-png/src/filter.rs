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
pub(crate) fn filter_image(
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
            FilterStrategy::MinSumAbs => choose_min_sum_abs(cur, prev, bpp, &mut scratch),
        };
        out.push(filter as u8);
        filter_row(filter, cur, prev, bpp, &mut scratch);
        out.extend_from_slice(&scratch);
        prev = cur;
    }
    out
}

/// Picks the filter with the lowest sum-of-absolute-residuals for one scanline.
fn choose_min_sum_abs(cur: &[u8], prev: &[u8], bpp: usize, scratch: &mut Vec<u8>) -> FilterType {
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

    /// Reconstructs a scanline from its filtered form (the decoder's inverse), to prove each filter
    /// is exactly invertible.
    fn reconstruct(filter: FilterType, filt: &[u8], prev: &[u8], bpp: usize) -> Vec<u8> {
        let mut cur = vec![0u8; filt.len()];
        for i in 0..filt.len() {
            let a = if i >= bpp { cur[i - bpp] } else { 0 };
            let b = prev[i];
            let c = if i >= bpp { prev[i - bpp] } else { 0 };
            cur[i] = match filter {
                FilterType::None => filt[i],
                FilterType::Sub => filt[i].wrapping_add(a),
                FilterType::Up => filt[i].wrapping_add(b),
                FilterType::Average => {
                    filt[i].wrapping_add(((u16::from(a) + u16::from(b)) / 2) as u8)
                }
                FilterType::Paeth => filt[i].wrapping_add(paeth(a, b, c)),
            };
        }
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
    fn min_sum_abs_prefers_flat_residuals() {
        // A horizontal gradient (each pixel = previous + k) filters to a constant under Sub, which
        // scores far below None.
        let row: Vec<u8> = (0..30u8).map(|i| i.wrapping_mul(3)).collect();
        let prev = vec![0u8; row.len()];
        let chosen = choose_min_sum_abs(&row, &prev, 1, &mut Vec::new());
        assert_eq!(chosen, FilterType::Sub);
    }
}
