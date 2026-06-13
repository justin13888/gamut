//! A small RGB preview/thumbnail for IFD 0, derived from the raw image.
//!
//! DNG stores a viewable preview in IFD 0 and the full-resolution raw in a sub-IFD. This produces a
//! simple, correct preview by collapsing each `2 × 2` block into one RGB pixel — for a CFA mosaic
//! that doubles as a trivial demosaic (averaging the sensels of each colour in the repeat tile),
//! for a linear image it box-averages the planes. It is intentionally basic (no proper demosaic,
//! gamma, or size cap); a higher-quality / JPEG-compressed preview is a later phase (see
//! `STATUS.md`).

use gamut_core::Dimensions;

use crate::raw::{RawImage, RawPhotometry, cfa_color};

/// Builds an 8-bit RGB preview from `raw`, returning its dimensions and interleaved `RGB8` samples.
///
/// The preview is `floor(width / 2) × floor(height / 2)` pixels (at least `1 × 1`).
#[must_use]
pub(crate) fn raw_preview(raw: &RawImage) -> (Dimensions, Vec<u8>) {
    match raw.photometry() {
        RawPhotometry::Cfa {
            repeat, pattern, ..
        } => cfa_preview(raw, *repeat, pattern),
        RawPhotometry::LinearRaw { planes } => linear_preview(raw, *planes),
    }
}

/// 8-bit scaling of a linear sample value given the black level and range.
fn scale8(value: u32, black: u32, range: u32) -> u8 {
    let scaled = u64::from(value.saturating_sub(black)) * 255 / u64::from(range);
    scaled.min(255) as u8
}

/// Collapses each CFA repeat tile into one RGB pixel by averaging each colour's sensels.
fn cfa_preview(raw: &RawImage, repeat: (u16, u16), pattern: &[u8]) -> (Dimensions, Vec<u8>) {
    let w = raw.dimensions().width as usize;
    let h = raw.dimensions().height as usize;
    let (rr, rc) = (repeat.0 as usize, repeat.1 as usize);
    let samples = raw.samples();
    let pw = (w / rc).max(1);
    let ph = (h / rr).max(1);
    let black = raw.black_level();
    let range = raw.white_level().saturating_sub(black).max(1);

    let mut out = Vec::with_capacity(pw * ph * 3);
    for py in 0..ph {
        for px in 0..pw {
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
                    let channel = match pattern[ty * rc + tx] {
                        cfa_color::RED => 0,
                        cfa_color::GREEN => 1,
                        cfa_color::BLUE => 2,
                        _ => continue, // non-RGB CFA colours don't map to the RGB preview
                    };
                    sum[channel] += u64::from(samples[sy * w + sx]);
                    count[channel] += 1;
                }
            }
            for ch in 0..3 {
                let avg = sum[ch].checked_div(count[ch]).unwrap_or(0) as u32;
                out.push(scale8(avg, black, range));
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

/// Box-averages each `2 × 2` block of a linear image into one RGB pixel.
fn linear_preview(raw: &RawImage, planes: u16) -> (Dimensions, Vec<u8>) {
    let w = raw.dimensions().width as usize;
    let h = raw.dimensions().height as usize;
    let p = usize::from(planes);
    let samples = raw.samples();
    let pw = (w / 2).max(1);
    let ph = (h / 2).max(1);
    let black = raw.black_level();
    let range = raw.white_level().saturating_sub(black).max(1);
    // Map preview R/G/B to source planes (the first three, clamped for <3-plane images).
    let chan = [0usize, 1.min(p - 1), 2.min(p - 1)];

    let mut out = Vec::with_capacity(pw * ph * 3);
    for py in 0..ph {
        for px in 0..pw {
            for &plane in &chan {
                let mut sum = 0u64;
                let mut count = 0u64;
                for dy in 0..2 {
                    let sy = py * 2 + dy;
                    if sy >= h {
                        break;
                    }
                    for dx in 0..2 {
                        let sx = px * 2 + dx;
                        if sx >= w {
                            break;
                        }
                        sum += u64::from(samples[(sy * w + sx) * p + plane]);
                        count += 1;
                    }
                }
                let avg = sum.checked_div(count).unwrap_or(0) as u32;
                out.push(scale8(avg, black, range));
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
    fn cfa_preview_halves_a_bayer_tile() {
        let pattern = vec![
            cfa_color::RED,
            cfa_color::GREEN,
            cfa_color::GREEN,
            cfa_color::BLUE,
        ];
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
        let (dims, rgb) = raw_preview(&raw);
        assert_eq!((dims.width, dims.height), (2, 2));
        assert_eq!(&rgb[0..3], &[200, 100, 50]);
        assert_eq!(rgb.len(), 2 * 2 * 3);
    }

    #[test]
    fn linear_preview_box_averages() {
        // 2x2 RGB image, white level 255; each pixel a flat colour, so the single preview pixel is
        // the average of the four.
        let samples = vec![
            100, 0, 0, 200, 0, 0, // row 0: two reds (100, 200)
            0, 60, 0, 0, 100, 0, // row 1: two greens
        ];
        let raw = RawImage::new_linear_raw(Dimensions::new(2, 2).unwrap(), 8, 3, samples).unwrap();
        let (dims, rgb) = raw_preview(&raw);
        assert_eq!((dims.width, dims.height), (1, 1));
        // R = avg(100,200,0,0)=75, G = avg(0,0,60,100)=40, B = 0.
        assert_eq!(rgb, vec![75, 40, 0]);
    }
}
