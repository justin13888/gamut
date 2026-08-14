//! BT.601 YCbCr ↔ RGB conversion and 4:2:0 chroma subsampling for VP8 (WebP lossy).
//!
//! VP8 codes images as YCbCr 4:2:0 (`color_space = 0`, RFC 6386 §9.2; the WebP container references
//! BT.601). The *signal range* is selected with [`ColorRange`] — the same CICP range flag used in
//! `colr` / AV1 headers, so callers carry one range type end to end:
//!
//! - [`ColorRange::Limited`] — studio swing (Y ∈ `16..=235`, chroma ∈ `16..=240`). **This is what
//!   the WebP ecosystem uses**: libwebp's `cwebp`/`dwebp`, browsers, and every standard decoder
//!   assume limited range, so WebP files must be encoded this way to render with correct colors. The
//!   limited-range path reproduces libwebp's exact integer math (`src/dsp/yuv.h` `VP8RGBToY/U/V` and
//!   `VP8YUVToR/G/B`), so gamut and libwebp agree per pixel.
//! - [`ColorRange::Full`] — full / "JFIF" swing (Y and chroma over the whole `0..=255` range), the
//!   JPEG convention. Kept for callers that genuinely want full-range BT.601; it is **not** what a
//!   WebP file should carry.
//!
//! Chroma is box-subsampled (2×2 average) on the way down and nearest-replicated on the way up — a
//! correct, simple pair; better resampling (fancy upsampling, sharp YUV) is a quality concern tracked
//! as issue #32, not a correctness one. This conversion is deliberately *not* on the VP8 codec's
//! bit-exact path (the codec operates on YCbCr planes directly); it backs the public RGB API.

use gamut_core::luminance::BT601_LUMA_WEIGHTS;
use gamut_core::{Dimensions, Error, Result};

use crate::cicp::ColorRange;
use crate::clip_pixel8;

/// Fixed-point fractional bits for the conversion coefficients.
///
/// The same scale as [`LUMA_FIX`], which is what lets the full-range luma row below come straight
/// from `gamut_core::luminance` rather than being restated here (pinned by a test).
const FIX: i32 = 16;

/// The full-range BT.601 luma row, widened once from the workspace's single authoritative
/// definition so the per-pixel expression stays integer-typed.
///
/// Only the *luma* row is shared: the chroma rows below, and every limited-range coefficient, are
/// libwebp's own studio-swing values with no counterpart in `gamut_core`.
const LUMA_R: i32 = BT601_LUMA_WEIGHTS[0] as i32;
/// Green's full-range BT.601 luma coefficient. See [`LUMA_R`].
const LUMA_G: i32 = BT601_LUMA_WEIGHTS[1] as i32;
/// Blue's full-range BT.601 luma coefficient. See [`LUMA_R`].
const LUMA_B: i32 = BT601_LUMA_WEIGHTS[2] as i32;
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

/// Converts one RGB triple to BT.601 YCbCr in the given [`ColorRange`] (each component `0..=255`).
///
/// # Examples
///
/// ```
/// use gamut_color::{rgb_to_ycbcr, ColorRange};
/// // Limited-range BT.601 (what WebP uses): black → luma 16, neutral chroma 128.
/// assert_eq!(rgb_to_ycbcr(0, 0, 0, ColorRange::Limited), (16, 128, 128));
/// ```
#[must_use]
pub fn rgb_to_ycbcr(r: u8, g: u8, b: u8, range: ColorRange) -> (u8, u8, u8) {
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    match range {
        ColorRange::Full => {
            let y = (LUMA_R * r + LUMA_G * g + LUMA_B * b + HALF) >> FIX;
            let cb = (-11059 * r - 21709 * g + 32768 * b + CHROMA_BIAS) >> FIX;
            let cr = (32768 * r - 27439 * g - 5329 * b + CHROMA_BIAS) >> FIX;
            (clip_pixel8(y), clip_pixel8(cb), clip_pixel8(cr))
        }
        // libwebp's per-pixel coefficients (src/dsp/yuv.h `VP8RGBToY/U/V`): studio swing, +16 luma
        // offset; chroma uses the same `(128 << FIX) + HALF` bias as the full-range path.
        ColorRange::Limited => {
            let y = (16839 * r + 33059 * g + 6420 * b + LUMA_BIAS_LIMITED + HALF) >> FIX;
            let cb = (-9719 * r - 19081 * g + 28800 * b + CHROMA_BIAS) >> FIX;
            let cr = (28800 * r - 24116 * g - 4684 * b + CHROMA_BIAS) >> FIX;
            (clip_pixel8(y), clip_pixel8(cb), clip_pixel8(cr))
        }
    }
}

/// Converts one BT.601 YCbCr triple in the given [`ColorRange`] back to RGB (each `0..=255`).
#[must_use]
pub fn ycbcr_to_rgb(y: u8, cb: u8, cr: u8, range: ColorRange) -> (u8, u8, u8) {
    match range {
        ColorRange::Full => {
            let y = i32::from(y);
            let cb = i32::from(cb) - 128;
            let cr = i32::from(cr) - 128;
            let r = y + ((91881 * cr + HALF) >> FIX);
            let g = y + ((-22554 * cb - 46802 * cr + HALF) >> FIX);
            let b = y + ((116130 * cb + HALF) >> FIX);
            (clip_pixel8(r), clip_pixel8(g), clip_pixel8(b))
        }
        // libwebp's exact per-pixel inverse (src/dsp/yuv.h `VP8YUVToR/G/B`): the studio-swing offsets
        // are folded into the additive constants, so the raw 0..=255 samples feed straight in.
        ColorRange::Limited => {
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
/// ([`ColorRange`]), not of the stored planes.
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
    /// Returns [`Error::InvalidInput`] if any plane length does not match the dimensions, or if
    /// the luma sample count overflows `usize`.
    pub fn new(width: u32, height: u32, y: Vec<u8>, u: Vec<u8>, v: Vec<u8>) -> Result<Self> {
        let luma = Dimensions { width, height }.num_pixels().ok_or_else(|| {
            Error::invalid_input(env!("CARGO_PKG_NAME"), "image dimensions overflow usize")
        })?;
        // Cannot overflow: each chroma plane has at most as many samples as the luma plane
        // (`ceil(d / 2) <= d` for d >= 1, and 0 for d == 0), and `luma` just fit `usize`.
        let chroma = Self::chroma_width(width) as usize * Self::chroma_height(height) as usize;
        if y.len() != luma || u.len() != chroma || v.len() != chroma {
            return Err(Error::invalid_input(
                env!("CARGO_PKG_NAME"),
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

    /// Converts an interleaved 8-bit RGB image to YCbCr 4:2:0 in the given [`ColorRange`],
    /// box-averaging each 2×2 block of chroma (partial edge blocks average only the pixels that exist).
    ///
    /// # Examples
    ///
    /// ```
    /// use gamut_color::{Yuv420, ColorRange};
    /// let rgb = vec![128u8; 4 * 4 * 3]; // 4×4 flat gray
    /// let yuv = Yuv420::from_rgb8(&rgb, 4, 4, ColorRange::Limited).expect("valid length");
    /// assert_eq!(yuv.y().len(), 16); // full-resolution luma
    /// assert_eq!(yuv.u().len(), 4); // 2×2-subsampled chroma
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `rgb.len() != width * height * 3`, if that product
    /// overflows `usize`, or if either dimension is 0.
    pub fn from_rgb8(rgb: &[u8], width: u32, height: u32, range: ColorRange) -> Result<Self> {
        let samples = Dimensions::new(width, height)?
            .sample_count(3)
            .ok_or_else(|| {
                Error::invalid_input(env!("CARGO_PKG_NAME"), "image dimensions overflow usize")
            })?;
        if rgb.len() != samples {
            return Err(Error::invalid_input(
                env!("CARGO_PKG_NAME"),
                "rgb buffer length does not match dimensions",
            ));
        }
        let (w, h) = (width as usize, height as usize);
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

    /// Converts back to an interleaved 8-bit RGB image in the given [`ColorRange`], nearest-replicating
    /// each chroma sample across its 2×2 luma block.
    #[must_use]
    pub fn to_rgb8(&self, range: ColorRange) -> Vec<u8> {
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
    use gamut_core::luminance::LUMA_FIX;

    use super::*;
    use crate::cicp::ColorRange::{Full, Limited};

    #[test]
    fn full_range_luma_row_comes_from_gamut_core_unchanged() {
        // The luma row is no longer written down here, so this pins the two things that make the
        // substitution safe: the shared fixed-point scale, and the exact coefficients that were
        // inlined before. A drift in either would silently reshade every full-range conversion.
        // FIX stays independent of LUMA_FIX because it also scales the chroma rows below, which
        // have no counterpart in gamut-core -- so the two are asserted equal rather than aliased.
        assert_eq!(FIX, LUMA_FIX as i32);
        assert_eq!([LUMA_R, LUMA_G, LUMA_B], [19_595, 38_470, 7_471]);

        // And the observable result: every value the old literals produced, still produced. The
        // sweep covers the whole cube coarsely plus the endpoints where rounding is decided.
        for r in (0..=255u8).step_by(17) {
            for g in (0..=255u8).step_by(17) {
                for b in (0..=255u8).step_by(17) {
                    let expected = (i32::from(r) * 19_595
                        + i32::from(g) * 38_470
                        + i32::from(b) * 7_471
                        + HALF)
                        >> FIX;
                    let (y, _, _) = rgb_to_ycbcr(r, g, b, Full);
                    assert_eq!(i32::from(y), expected, "luma drifted at ({r}, {g}, {b})");
                }
            }
        }
    }

    #[test]
    fn full_range_color_anchors() {
        // Standard JFIF/BT.601 full-range values: black/white span the whole luma range.
        assert_eq!(rgb_to_ycbcr(0, 0, 0, Full), (0, 128, 128));
        assert_eq!(rgb_to_ycbcr(255, 255, 255, Full), (255, 128, 128));
        assert_eq!(rgb_to_ycbcr(255, 0, 0, Full), (76, 85, 255));
        let (y, cb, cr) = rgb_to_ycbcr(128, 128, 128, Full);
        assert_eq!((cb, cr), (128, 128));
        assert!((i32::from(y) - 128).abs() <= 1);
        // Neutral gray inverts to itself. Every channel lands on 128 — away from the 0/255 clamp —
        // so the `+ HALF` rounding term of each `ycbcr_to_rgb` component is observable (a mutated
        // `- HALF` shifts each result to 127).
        assert_eq!(ycbcr_to_rgb(128, 128, 128, Full), (128, 128, 128));
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

    #[test]
    fn rejects_overflowing_dimensions() {
        // Near-max dimensions must yield Err, not an overflow panic (debug) or a wrapped length
        // check (32-bit release): width * height * 3 exceeds usize even on 64-bit targets.
        assert!(Yuv420::from_rgb8(&[], u32::MAX, u32::MAX, Limited).is_err());
        assert!(Yuv420::new(u32::MAX, u32::MAX, vec![], vec![], vec![]).is_err());
    }

    #[test]
    fn vp8_clip8_fast_path_and_clamps() {
        // In-range values (`v & !MASK2 == 0`) take the `>> FIX2` fast path; out-of-range values clamp
        // hard to 0 (low) or 255 (high). The negative case pins `v < 0` against `v == 0`.
        assert_eq!(vp8_clip8(0), 0);
        assert_eq!(vp8_clip8(640), 10); // 640 >> 6
        assert_eq!(vp8_clip8(MASK2), (MASK2 >> FIX2) as u8); // largest in-range value (= 255)
        assert_eq!(vp8_clip8(MASK2 + 1), 255); // first out-of-range high
        assert_eq!(vp8_clip8(-1), 0); // out-of-range low
    }

    #[test]
    fn box_subsample_matches_reference_for_varying_image() {
        // A spatially-varying 5x3 image (odd dims ⇒ partial edge blocks, incl. a 1-pixel corner) so
        // the chroma box-average exercises the source coordinates `cx*2`/`cy*2` and the per-block
        // `count` rounding. An independent reference re-derives the expected U/V; any mutated
        // coordinate or rounding in `from_rgb8` diverges from it.
        let (w, h) = (5usize, 3usize);
        let mut rgb = vec![0u8; w * h * 3];
        for y in 0..h {
            for x in 0..w {
                let i = (y * w + x) * 3;
                rgb[i] = (x * 50).min(255) as u8;
                rgb[i + 1] = (y * 90).min(255) as u8;
                rgb[i + 2] = ((x + y) * 35).min(255) as u8;
            }
        }
        let yuv = Yuv420::from_rgb8(&rgb, w as u32, h as u32, Full).unwrap();
        // 5x3 luma ⇒ chroma is ceil(5/2) x ceil(3/2) = 3 x 2, as literals — re-deriving them from
        // chroma_width/chroma_height here would let a broken round-up hide. The odd sizes pin it.
        assert_eq!(Yuv420::chroma_width(w as u32), 3);
        assert_eq!(Yuv420::chroma_height(h as u32), 2);
        assert_eq!(yuv.y().len(), 15);
        assert_eq!((yuv.u().len(), yuv.v().len()), (6, 6));
        let (cw, ch) = (3usize, 2usize);
        for cy in 0..ch {
            for cx in 0..cw {
                let (mut su, mut sv, mut count) = (0u32, 0u32, 0u32);
                for dy in 0..2 {
                    for dx in 0..2 {
                        let (px, py) = (cx * 2 + dx, cy * 2 + dy);
                        if px < w && py < h {
                            let i = (py * w + px) * 3;
                            let (_, cb, cr) = rgb_to_ycbcr(rgb[i], rgb[i + 1], rgb[i + 2], Full);
                            su += u32::from(cb);
                            sv += u32::from(cr);
                            count += 1;
                        }
                    }
                }
                assert_eq!(
                    yuv.u()[cy * cw + cx],
                    ((su + count / 2) / count) as u8,
                    "u[{cx},{cy}]"
                );
                assert_eq!(
                    yuv.v()[cy * cw + cx],
                    ((sv + count / 2) / count) as u8,
                    "v[{cx},{cy}]"
                );
            }
        }
    }

    #[test]
    fn to_rgb8_matches_reference_upsampling() {
        // Distinct per-position Y/U/V so the nearest-chroma index `(py/2)*cw + (px/2)` and the luma
        // index `py*w + px` both matter; an independent reference upsamples identically. An empty
        // `vec![]` body, a mutated index, or wrong index arithmetic all diverge.
        let (w, h) = (5u32, 3u32);
        let cw = Yuv420::chroma_width(w);
        let chh = Yuv420::chroma_height(h);
        let y: Vec<u8> = (0..w * h).map(|i| (i * 7 % 251) as u8).collect();
        let u: Vec<u8> = (0..cw * chh).map(|i| (30 + i * 17 % 200) as u8).collect();
        let v: Vec<u8> = (0..cw * chh).map(|i| (220 - i * 13 % 200) as u8).collect();
        let yuv = Yuv420::new(w, h, y.clone(), u.clone(), v.clone()).unwrap();
        assert_eq!((yuv.width(), yuv.height()), (w, h));
        let out = yuv.to_rgb8(Full);
        assert_eq!(out.len(), (w * h * 3) as usize);
        let (wu, cwu) = (w as usize, cw as usize);
        for py in 0..h as usize {
            for px in 0..wu {
                let ci = (py / 2) * cwu + (px / 2);
                let (r, g, b) = ycbcr_to_rgb(y[py * wu + px], u[ci], v[ci], Full);
                let o = (py * wu + px) * 3;
                assert_eq!(
                    (out[o], out[o + 1], out[o + 2]),
                    (r, g, b),
                    "pixel ({px},{py})"
                );
            }
        }
    }
}
