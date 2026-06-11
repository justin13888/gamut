//! BT.601 YCbCr ↔ RGB conversion and 4:2:0 chroma subsampling for VP8 (WebP lossy).
//!
//! VP8 codes images as YCbCr 4:2:0 (`color_space = 0`, RFC 6386 §9.2; the WebP container references
//! BT.601). The *signal range* is selectable via [`Bt601Range`]:
//!
//! - [`Bt601Range::Limited`] — studio swing (Y ∈ `16..=235`, chroma ∈ `16..=240`). **This is what
//!   the WebP ecosystem uses**: libwebp's `cwebp`/`dwebp`, browsers, and every standard decoder
//!   assume limited range, so WebP files must be encoded this way to render with correct colors. The
//!   limited-range path reproduces libwebp's exact integer math (`src/dsp/yuv.h` `VP8RGBToY/U/V` and
//!   `VP8YUVToR/G/B`), so gamut and libwebp agree per pixel.
//! - [`Bt601Range::Full`] — full / "JFIF" swing (Y and chroma over the whole `0..=255` range), the
//!   JPEG convention. Kept for callers that genuinely want full-range BT.601; it is **not** what a
//!   WebP file should carry.
//!
//! Chroma is box-subsampled (2×2 average) on the way down and nearest-replicated on the way up — a
//! correct, simple pair; better resampling (fancy upsampling, sharp YUV) is a quality concern tracked
//! as issue #32, not a correctness one. This conversion is deliberately *not* on the VP8 codec's
//! bit-exact path (the codec operates on YCbCr planes directly); it backs the public RGB API.

use gamut_core::{Error, Result};

/// Fixed-point fractional bits for the conversion coefficients.
const FIX: i32 = 16;
/// Rounding addend (`0.5` in the fixed-point scale).
const HALF: i32 = 1 << (FIX - 1);
/// The chroma offset (`128`) pre-scaled to the fixed-point domain, with rounding folded in.
const CHROMA_BIAS: i32 = (128 << FIX) + HALF;
/// The limited-range luma offset (`16`) pre-scaled to the fixed-point domain.
const LUMA_BIAS_LIMITED: i32 = 16 << FIX;
/// Fractional bits of libwebp's YUV→RGB inverse (`YUV_FIX2` in `src/dsp/yuv.h`).
const FIX2: i32 = 6;
/// In-range mask for libwebp's `VP8Clip8` fast path (`YUV_MASK2 = (256 << YUV_FIX2) - 1`).
const MASK2: i32 = (256 << FIX2) - 1;

/// The signal range of a BT.601 YCbCr encoding. See the [module docs](self) for which to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bt601Range {
    /// Full / "JFIF" range: luma and chroma span the entire `0..=255` byte range (the JPEG
    /// convention). Not what a WebP file should carry.
    Full,
    /// Limited / "studio" range: luma in `16..=235`, chroma in `16..=240`. The range libwebp and the
    /// WebP/VP8 ecosystem (browsers, `dwebp`, …) assume, so files encoded this way render correctly.
    Limited,
}

/// Saturates a fixed-point-derived integer to the unsigned 8-bit range.
fn clip8(x: i32) -> u8 {
    x.clamp(0, 255) as u8
}

/// libwebp's `MultHi` (`src/dsp/yuv.h`): the high bits of a fixed-point product, `(v * coeff) >> 8`.
fn mult_hi(v: i32, coeff: i32) -> i32 {
    (v * coeff) >> 8
}

/// libwebp's `VP8Clip8` (`src/dsp/yuv.h`): a `>> FIX2` shift for in-range values, else a hard clamp.
fn vp8_clip8(v: i32) -> u8 {
    if v & !MASK2 == 0 {
        (v >> FIX2) as u8
    } else if v < 0 {
        0
    } else {
        255
    }
}

/// Converts one RGB triple to BT.601 YCbCr in the given [`Bt601Range`] (each component `0..=255`).
#[must_use]
pub fn rgb_to_ycbcr(r: u8, g: u8, b: u8, range: Bt601Range) -> (u8, u8, u8) {
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    match range {
        Bt601Range::Full => {
            let y = (19595 * r + 38470 * g + 7471 * b + HALF) >> FIX;
            let cb = (-11059 * r - 21709 * g + 32768 * b + CHROMA_BIAS) >> FIX;
            let cr = (32768 * r - 27439 * g - 5329 * b + CHROMA_BIAS) >> FIX;
            (clip8(y), clip8(cb), clip8(cr))
        }
        // libwebp's per-pixel coefficients (src/dsp/yuv.h `VP8RGBToY/U/V`): studio swing, +16 luma
        // offset; chroma uses the same `(128 << FIX) + HALF` bias as the full-range path.
        Bt601Range::Limited => {
            let y = (16839 * r + 33059 * g + 6420 * b + LUMA_BIAS_LIMITED + HALF) >> FIX;
            let cb = (-9719 * r - 19081 * g + 28800 * b + CHROMA_BIAS) >> FIX;
            let cr = (28800 * r - 24116 * g - 4684 * b + CHROMA_BIAS) >> FIX;
            (clip8(y), clip8(cb), clip8(cr))
        }
    }
}

/// Converts one BT.601 YCbCr triple in the given [`Bt601Range`] back to RGB (each `0..=255`).
#[must_use]
pub fn ycbcr_to_rgb(y: u8, cb: u8, cr: u8, range: Bt601Range) -> (u8, u8, u8) {
    match range {
        Bt601Range::Full => {
            let y = i32::from(y);
            let cb = i32::from(cb) - 128;
            let cr = i32::from(cr) - 128;
            let r = y + ((91881 * cr + HALF) >> FIX);
            let g = y + ((-22554 * cb - 46802 * cr + HALF) >> FIX);
            let b = y + ((116130 * cb + HALF) >> FIX);
            (clip8(r), clip8(g), clip8(b))
        }
        // libwebp's exact per-pixel inverse (src/dsp/yuv.h `VP8YUVToR/G/B`): the studio-swing offsets
        // are folded into the additive constants, so the raw 0..=255 samples feed straight in.
        Bt601Range::Limited => {
            let (y, cb, cr) = (i32::from(y), i32::from(cb), i32::from(cr));
            let yy = mult_hi(y, 19077);
            let r = vp8_clip8(yy + mult_hi(cr, 26149) - 14234);
            let g = vp8_clip8(yy - mult_hi(cb, 6419) - mult_hi(cr, 13320) + 8708);
            let b = vp8_clip8(yy + mult_hi(cb, 33050) - 17685);
            (r, g, b)
        }
    }
}

/// A BT.601 YCbCr image in 4:2:0 layout: a `width × height` luma plane and two
/// `chroma_width × chroma_height` chroma planes, all row-major 8-bit, where the chroma dimensions are
/// `ceil(width / 2)` and `ceil(height / 2)`. The signal range is a property of the conversion
/// ([`Bt601Range`]), not of the stored planes.
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

    /// Converts an interleaved 8-bit RGB image to YCbCr 4:2:0 in the given [`Bt601Range`],
    /// box-averaging each 2×2 block of chroma (partial edge blocks average only the pixels that exist).
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `rgb.len() != width * height * 3`, or either dimension is 0.
    pub fn from_rgb8(rgb: &[u8], width: u32, height: u32, range: Bt601Range) -> Result<Self> {
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
            let (yy, cb, cr) = rgb_to_ycbcr(rgb[i * 3], rgb[i * 3 + 1], rgb[i * 3 + 2], range);
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

    /// Converts back to an interleaved 8-bit RGB image in the given [`Bt601Range`], nearest-replicating
    /// each chroma sample across its 2×2 luma block.
    #[must_use]
    pub fn to_rgb8(&self, range: Bt601Range) -> Vec<u8> {
        let (w, h) = (self.width as usize, self.height as usize);
        let cw = Self::chroma_width(self.width) as usize;
        let mut out = vec![0u8; w * h * 3];
        for py in 0..h {
            for px in 0..w {
                let ci = (py / 2) * cw + (px / 2);
                let (r, g, b) = ycbcr_to_rgb(self.y[py * w + px], self.u[ci], self.v[ci], range);
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

    use Bt601Range::{Full, Limited};

    #[test]
    fn full_range_color_anchors() {
        // Standard JFIF/BT.601 full-range values: black/white span the whole luma range.
        assert_eq!(rgb_to_ycbcr(0, 0, 0, Full), (0, 128, 128));
        assert_eq!(rgb_to_ycbcr(255, 255, 255, Full), (255, 128, 128));
        assert_eq!(rgb_to_ycbcr(255, 0, 0, Full), (76, 85, 255));
        let (y, cb, cr) = rgb_to_ycbcr(128, 128, 128, Full);
        assert_eq!((cb, cr), (128, 128));
        assert!((i32::from(y) - 128).abs() <= 1);
    }

    #[test]
    fn limited_range_matches_libwebp_anchors() {
        // Studio swing: black→16, white→235, neutral chroma 128. Red matches libwebp's per-pixel
        // VP8RGBToY/U/V (src/dsp/yuv.h) exactly, pinning the port independent of the FFI oracle.
        assert_eq!(rgb_to_ycbcr(0, 0, 0, Limited), (16, 128, 128));
        assert_eq!(rgb_to_ycbcr(255, 255, 255, Limited), (235, 128, 128));
        assert_eq!(rgb_to_ycbcr(255, 0, 0, Limited), (82, 90, 240));
        // The inverse round-trips that red back to (near) pure red via libwebp's VP8YUVToR/G/B.
        let (r, g, b) = ycbcr_to_rgb(82, 90, 240, Limited);
        assert!(
            r >= 254 && g <= 2 && b <= 2,
            "limited red inverse = ({r},{g},{b})"
        );
    }

    #[test]
    fn limited_luma_stays_in_studio_range() {
        // Every limited-range luma sample lands in [16, 235]; the full-range path reaches 0 and 255.
        for r in (0..=255).step_by(17) {
            for g in (0..=255).step_by(17) {
                let (yl, ..) = rgb_to_ycbcr(r, g, 128, Limited);
                assert!(
                    (16..=235).contains(&yl),
                    "limited Y {yl} out of studio range"
                );
            }
        }
        assert_eq!(rgb_to_ycbcr(0, 0, 0, Full).0, 0);
        assert_eq!(rgb_to_ycbcr(255, 255, 255, Full).0, 255);
    }

    #[test]
    fn pixel_roundtrip_within_tolerance() {
        // The forward/inverse pair recovers RGB within a few units (no subsampling), in both ranges.
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
        for range in [Full, Limited] {
            for (r, g, b) in colors {
                let (y, cb, cr) = rgb_to_ycbcr(r, g, b, range);
                let (r2, g2, b2) = ycbcr_to_rgb(y, cb, cr, range);
                let err = (i32::from(r) - i32::from(r2)).abs().max(
                    (i32::from(g) - i32::from(g2))
                        .abs()
                        .max((i32::from(b) - i32::from(b2)).abs()),
                );
                assert!(
                    err <= 4,
                    "{range:?} color ({r},{g},{b}) round-trip error {err}"
                );
            }
        }
    }

    #[test]
    fn flat_image_roundtrips_through_420() {
        // A constant-color image has constant chroma, so 4:2:0 subsampling is lossless on chroma and
        // the whole image round-trips to within the per-pixel conversion tolerance, in both ranges.
        let rgb: Vec<u8> = [90u8, 140, 200].repeat(7 * 5);
        for range in [Full, Limited] {
            let yuv = Yuv420::from_rgb8(&rgb, 7, 5, range).unwrap();
            let back = yuv.to_rgb8(range);
            for (a, b) in rgb.iter().zip(&back) {
                assert!(
                    (i32::from(*a) - i32::from(*b)).abs() <= 4,
                    "{range:?} round-trip"
                );
            }
        }
    }

    #[test]
    fn chroma_dimensions_round_up_for_odd_sizes() {
        let yuv = Yuv420::from_rgb8(&[0u8; 5 * 3 * 3], 5, 3, Limited).unwrap();
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
        let yuv = Yuv420::from_rgb8(&rgb, 2, 2, Full).unwrap();
        assert_eq!((yuv.u().len(), yuv.v().len()), (1, 1));
        let mut su = 0u32;
        let mut sv = 0u32;
        for &(r, g, b) in &[(255u8, 0u8, 0u8), (0, 255, 0), (0, 0, 255), (255, 255, 255)] {
            let (_, cb, cr) = rgb_to_ycbcr(r, g, b, Full);
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
        assert!(Yuv420::from_rgb8(&[0, 1, 2, 3], 1, 1, Limited).is_err());
        assert!(Yuv420::from_rgb8(&[], 0, 1, Limited).is_err());
    }
}
