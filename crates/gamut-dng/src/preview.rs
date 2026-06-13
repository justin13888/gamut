//! A small RGB preview/thumbnail for IFD 0, derived from the raw CFA mosaic.
//!
//! DNG stores a viewable preview in IFD 0 and the full-resolution raw in a sub-IFD. This produces a
//! simple, correct preview by collapsing each CFA repeat tile into one RGB pixel — averaging the
//! sensels of each colour in the tile and scaling the linear range to 8 bits. It is intentionally
//! basic (no proper demosaic, gamma, or size cap); a higher-quality / JPEG-compressed preview is a
//! later phase (see `STATUS.md`).

use gamut_core::Dimensions;

use crate::raw::{RawImage, cfa_color};

/// Builds an 8-bit RGB preview from `raw` by box-averaging each CFA repeat tile into one pixel.
///
/// Returns the preview dimensions and its interleaved `RGB8` samples. The preview is
/// `floor(width / cols) × floor(height / rows)` pixels (at least `1 × 1`).
#[must_use]
pub(crate) fn cfa_preview(raw: &RawImage) -> (Dimensions, Vec<u8>) {
    let w = raw.dimensions().width as usize;
    let h = raw.dimensions().height as usize;
    let (rr, rc) = raw.cfa_repeat();
    let (rr, rc) = (rr as usize, rc as usize);
    let pattern = raw.cfa_pattern();
    let samples = raw.samples();

    let pw = (w / rc).max(1);
    let ph = (h / rr).max(1);
    let black = raw.black_level();
    let range = raw.white_level().saturating_sub(black).max(1);

    let mut out = Vec::with_capacity(pw * ph * 3);
    for py in 0..ph {
        for px in 0..pw {
            // Accumulate each colour's sensels within this repeat tile.
            let mut sum = [0u64; 3];
            let mut count = [0u64; 3];
            for ty in 0..rr {
                let sy = py * rr + ty;
                if sy >= h {
                    break;
                }
                for tx in 0..rc {
                    let sx = px * rc + tx;
                    if sx >= w {
                        break;
                    }
                    let color = pattern[ty * rc + tx];
                    let channel = match color {
                        cfa_color::RED => 0,
                        cfa_color::GREEN => 1,
                        cfa_color::BLUE => 2,
                        _ => continue, // non-RGB CFA colours don't map to the RGB preview
                    };
                    let v = u64::from(samples[sy * w + sx]);
                    sum[channel] += v;
                    count[channel] += 1;
                }
            }
            for ch in 0..3 {
                let avg = sum[ch].checked_div(count[ch]).unwrap_or(0) as u32;
                let scaled = u64::from(avg.saturating_sub(black)) * 255 / u64::from(range);
                out.push(scaled.min(255) as u8);
            }
        }
    }

    (
        Dimensions {
            width: pw as u32,
            height: ph as u32,
        },
        out,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_halves_a_bayer_tile() {
        // 4x4 RGGB mosaic, 8-bit. Each 2x2 tile -> 1 RGB pixel; preview is 2x2.
        let pattern = vec![
            cfa_color::RED,
            cfa_color::GREEN,
            cfa_color::GREEN,
            cfa_color::BLUE,
        ];
        // Fill so each tile has R=200, G=100/100, B=50 (white level 255).
        let mut samples = vec![0u16; 16];
        for ty in 0..2usize {
            for tx in 0..2usize {
                for r in 0..2usize {
                    for c in 0..2usize {
                        let (y, x) = (ty * 2 + r, tx * 2 + c);
                        samples[y * 4 + x] = match pattern[r * 2 + c] {
                            cfa_color::RED => 200,
                            cfa_color::GREEN => 100,
                            _ => 50,
                        };
                    }
                }
            }
        }
        let raw =
            RawImage::new_cfa(Dimensions::new(4, 4).unwrap(), 8, (2, 2), pattern, samples).unwrap();
        let (dims, rgb) = cfa_preview(&raw);
        assert_eq!((dims.width, dims.height), (2, 2));
        // First preview pixel: R=200, G=100, B=50 scaled by 255/255.
        assert_eq!(&rgb[0..3], &[200, 100, 50]);
        assert_eq!(rgb.len(), 2 * 2 * 3);
    }
}
