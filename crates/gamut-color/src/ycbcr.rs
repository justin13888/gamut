//! BT.601 full-range YCbCr ↔ RGB conversion and 4:2:0 chroma subsampling for VP8 (WebP lossy).
//!
//! VP8 codes images as full-range YCbCr 4:2:0 (`color_space = 0`, RFC 6386 §9.2; the WebP container
//! references BT.601). This module converts interleaved 8-bit RGB to a [`Yuv420`] buffer (full-res
//! luma, quarter-res chroma) and back, using the JFIF/BT.601 full-range matrix in 16-bit fixed point.
//!
//! Chroma is box-subsampled (2×2 average) on the way down and nearest-replicated on the way up — a
//! correct, simple pair; better resampling (fancy upsampling, sharp YUV) is a quality concern tracked
//! as issue #32, not a correctness one. This conversion is deliberately *not* on the VP8 codec's
//! bit-exact path (the codec operates on YCbCr planes directly); it backs the public RGB API, so it is
//! validated by bounded round-trip rather than against any specific reference.

use gamut_core::{Error, Result};

/// Fixed-point fractional bits for the conversion coefficients.
const FIX: i32 = 16;
/// Rounding addend (`0.5` in the fixed-point scale).
const HALF: i32 = 1 << (FIX - 1);
/// The chroma offset (`128`) pre-scaled to the fixed-point domain, with rounding folded in.
const CHROMA_BIAS: i32 = (128 << FIX) + HALF;

/// Saturates a fixed-point-derived integer to the unsigned 8-bit range.
fn clip8(x: i32) -> u8 {
    x.clamp(0, 255) as u8
}

/// Converts one full-range RGB triple to BT.601 YCbCr (each `0..=255`).
#[must_use]
pub fn rgb_to_ycbcr(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    let y = (19595 * r + 38470 * g + 7471 * b + HALF) >> FIX;
    let cb = (-11059 * r - 21709 * g + 32768 * b + CHROMA_BIAS) >> FIX;
    let cr = (32768 * r - 27439 * g - 5329 * b + CHROMA_BIAS) >> FIX;
    (clip8(y), clip8(cb), clip8(cr))
}

/// Converts one BT.601 YCbCr triple back to full-range RGB (each `0..=255`).
#[must_use]
pub fn ycbcr_to_rgb(y: u8, cb: u8, cr: u8) -> (u8, u8, u8) {
    let y = i32::from(y);
    let cb = i32::from(cb) - 128;
    let cr = i32::from(cr) - 128;
    let r = y + ((91881 * cr + HALF) >> FIX);
    let g = y + ((-22554 * cb - 46802 * cr + HALF) >> FIX);
    let b = y + ((116130 * cb + HALF) >> FIX);
    (clip8(r), clip8(g), clip8(b))
}

/// A full-range BT.601 YCbCr image in 4:2:0 layout: a `width × height` luma plane and two
/// `chroma_width × chroma_height` chroma planes, all row-major 8-bit, where the chroma dimensions are
/// `ceil(width / 2)` and `ceil(height / 2)`.
#[derive(Debug, Clone)]
pub struct Yuv420 {
    width: u32,
    height: u32,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

impl Yuv420 {
    /// Chroma plane width, `ceil(width / 2)`.
    #[must_use]
    pub fn chroma_width(width: u32) -> u32 {
        width.div_ceil(2)
    }

    /// Chroma plane height, `ceil(height / 2)`.
    #[must_use]
    pub fn chroma_height(height: u32) -> u32 {
        height.div_ceil(2)
    }

    /// Builds a buffer from existing planes (e.g. a decoder's output), validating their lengths.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if any plane length does not match the dimensions.
    pub fn new(width: u32, height: u32, y: Vec<u8>, u: Vec<u8>, v: Vec<u8>) -> Result<Self> {
        let luma = width as usize * height as usize;
        let chroma = Self::chroma_width(width) as usize * Self::chroma_height(height) as usize;
        if y.len() != luma || u.len() != chroma || v.len() != chroma {
            return Err(Error::InvalidInput(
                "YUV plane length does not match dimensions",
            ));
        }
        Ok(Self {
            width,
            height,
            y,
            u,
            v,
        })
    }

    /// Converts an interleaved 8-bit RGB image to full-range YCbCr 4:2:0, box-averaging each 2×2 block
    /// of chroma (partial edge blocks average only the pixels that exist).
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `rgb.len() != width * height * 3`, or either dimension is 0.
    pub fn from_rgb8(rgb: &[u8], width: u32, height: u32) -> Result<Self> {
        let (w, h) = (width as usize, height as usize);
        if width == 0 || height == 0 || rgb.len() != w * h * 3 {
            return Err(Error::InvalidInput(
                "rgb buffer length does not match dimensions",
            ));
        }
        // Full-resolution luma, plus full-resolution chroma we then average down.
        let mut y = vec![0u8; w * h];
        let mut cb_full = vec![0u8; w * h];
        let mut cr_full = vec![0u8; w * h];
        for i in 0..w * h {
            let (yy, cb, cr) = rgb_to_ycbcr(rgb[i * 3], rgb[i * 3 + 1], rgb[i * 3 + 2]);
            y[i] = yy;
            cb_full[i] = cb;
            cr_full[i] = cr;
        }

        let cw = Self::chroma_width(width) as usize;
        let ch = Self::chroma_height(height) as usize;
        let mut u = vec![0u8; cw * ch];
        let mut v = vec![0u8; cw * ch];
        for cy in 0..ch {
            for cx in 0..cw {
                let (mut su, mut sv, mut count) = (0u32, 0u32, 0u32);
                for dy in 0..2 {
                    for dx in 0..2 {
                        let (px, py) = (cx * 2 + dx, cy * 2 + dy);
                        if px < w && py < h {
                            su += u32::from(cb_full[py * w + px]);
                            sv += u32::from(cr_full[py * w + px]);
                            count += 1;
                        }
                    }
                }
                u[cy * cw + cx] = ((su + count / 2) / count) as u8;
                v[cy * cw + cx] = ((sv + count / 2) / count) as u8;
            }
        }
        Ok(Self {
            width,
            height,
            y,
            u,
            v,
        })
    }

    /// Converts back to an interleaved 8-bit RGB image, nearest-replicating each chroma sample across
    /// its 2×2 luma block.
    #[must_use]
    pub fn to_rgb8(&self) -> Vec<u8> {
        let (w, h) = (self.width as usize, self.height as usize);
        let cw = Self::chroma_width(self.width) as usize;
        let mut out = vec![0u8; w * h * 3];
        for py in 0..h {
            for px in 0..w {
                let ci = (py / 2) * cw + (px / 2);
                let (r, g, b) = ycbcr_to_rgb(self.y[py * w + px], self.u[ci], self.v[ci]);
                let o = (py * w + px) * 3;
                out[o] = r;
                out[o + 1] = g;
                out[o + 2] = b;
            }
        }
        out
    }

    /// Image width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Image height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The full-resolution luma plane (`width * height` samples, row-major).
    #[must_use]
    pub fn y(&self) -> &[u8] {
        &self.y
    }

    /// The subsampled Cb (U) plane.
    #[must_use]
    pub fn u(&self) -> &[u8] {
        &self.u
    }

    /// The subsampled Cr (V) plane.
    #[must_use]
    pub fn v(&self) -> &[u8] {
        &self.v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_color_anchors() {
        // Standard JFIF/BT.601 full-range values.
        assert_eq!(rgb_to_ycbcr(0, 0, 0), (0, 128, 128));
        assert_eq!(rgb_to_ycbcr(255, 255, 255), (255, 128, 128));
        assert_eq!(rgb_to_ycbcr(255, 0, 0), (76, 85, 255));
        // Grayscale stays neutral chroma and maps luma to (near) the gray level.
        let (y, cb, cr) = rgb_to_ycbcr(128, 128, 128);
        assert_eq!((cb, cr), (128, 128));
        assert!((i32::from(y) - 128).abs() <= 1);
    }

    #[test]
    fn pixel_roundtrip_within_tolerance() {
        // The fixed-point forward/inverse pair recovers RGB within a couple of units (no subsampling).
        let colors = [
            (0, 0, 0),
            (255, 255, 255),
            (255, 0, 0),
            (0, 255, 0),
            (0, 0, 255),
            (10, 200, 90),
            (123, 45, 67),
            (200, 200, 50),
            (17, 17, 200),
        ];
        for (r, g, b) in colors {
            let (y, cb, cr) = rgb_to_ycbcr(r, g, b);
            let (r2, g2, b2) = ycbcr_to_rgb(y, cb, cr);
            let err = (i32::from(r) - i32::from(r2)).abs().max(
                (i32::from(g) - i32::from(g2))
                    .abs()
                    .max((i32::from(b) - i32::from(b2)).abs()),
            );
            assert!(err <= 3, "color ({r},{g},{b}) round-trip error {err}");
        }
    }

    #[test]
    fn flat_image_roundtrips_exactly_through_420() {
        // A constant-color image has constant chroma, so 4:2:0 subsampling is lossless on chroma and
        // the whole image round-trips to within the per-pixel conversion tolerance.
        let rgb: Vec<u8> = [90u8, 140, 200].repeat(7 * 5);
        let yuv = Yuv420::from_rgb8(&rgb, 7, 5).unwrap();
        let back = yuv.to_rgb8();
        for (a, b) in rgb.iter().zip(&back) {
            assert!((i32::from(*a) - i32::from(*b)).abs() <= 3);
        }
    }

    #[test]
    fn chroma_dimensions_round_up_for_odd_sizes() {
        let yuv = Yuv420::from_rgb8(&[0u8; 5 * 3 * 3], 5, 3).unwrap();
        // 5x3 luma; chroma is ceil(5/2) x ceil(3/2) = 3 x 2.
        assert_eq!(yuv.y().len(), 15);
        assert_eq!((yuv.u().len(), yuv.v().len()), (6, 6));
        assert_eq!(Yuv420::chroma_width(5), 3);
        assert_eq!(Yuv420::chroma_height(3), 2);
    }

    #[test]
    fn box_subsample_averages_the_block() {
        // A 2x2 image whose Cb/Cr differ per pixel collapses to one chroma sample = the 4-pixel
        // average. Use pure primaries so the chroma spread is large and the averaging is visible.
        let rgb = [
            255, 0, 0, // red
            0, 255, 0, // green
            0, 0, 255, // blue
            255, 255, 255, // white
        ];
        let yuv = Yuv420::from_rgb8(&rgb, 2, 2).unwrap();
        assert_eq!((yuv.u().len(), yuv.v().len()), (1, 1));
        let mut su = 0u32;
        let mut sv = 0u32;
        for &(r, g, b) in &[(255u8, 0u8, 0u8), (0, 255, 0), (0, 0, 255), (255, 255, 255)] {
            let (_, cb, cr) = rgb_to_ycbcr(r, g, b);
            su += u32::from(cb);
            sv += u32::from(cr);
        }
        assert_eq!(yuv.u()[0], ((su + 2) / 4) as u8);
        assert_eq!(yuv.v()[0], ((sv + 2) / 4) as u8);
    }

    #[test]
    fn new_validates_plane_lengths() {
        assert!(Yuv420::new(4, 4, vec![0; 16], vec![0; 4], vec![0; 4]).is_ok());
        assert!(Yuv420::new(4, 4, vec![0; 16], vec![0; 3], vec![0; 4]).is_err());
        assert!(Yuv420::new(4, 4, vec![0; 15], vec![0; 4], vec![0; 4]).is_err());
    }

    #[test]
    fn rejects_bad_rgb_length() {
        assert!(Yuv420::from_rgb8(&[0, 1, 2, 3], 1, 1).is_err());
        assert!(Yuv420::from_rgb8(&[], 0, 1).is_err());
    }
}
