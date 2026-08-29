//! The high-bit-depth planar buffer: [`Planar16`], the 10/12-bit sibling of
//! [`Planar8`](crate::Planar8).
//!
//! Same geometry model, same plane order, wider samples. It exists rather than a generic
//! `Planar<T>` because the two are used at different boundaries — 8-bit encoding is the common
//! path and pays no `u16` widening for it — and because a high-bit-depth buffer carries one thing
//! an 8-bit buffer cannot need: the [`BitDepth`] its samples are coded at. `u16` is storage, `10`
//! or `12` is meaning, and every sample is validated against it at construction.

use gamut_core::{Dimensions, Error, ImageRef, Result, Rgb16, Rgba16};

use crate::format::{BitDepth, ChromaSubsampling};
use crate::ycbcr::RgbToYcbcr;

/// How far a full-range 16-bit sample is shifted down to reach `bit_depth`.
///
/// [`gamut_core`]'s `u16` pixel layouts carry samples on the canonical full 16-bit scale, while a
/// codec's *coded* depth is a separate concern — so narrowing is a right shift, and it **truncates**
/// rather than rounds. Truncation is what keeps the mapping a pure prefix of the sample: the coded
/// value is literally the top `bit_depth` bits, so the same source narrowed to 10 and to 12 bits
/// agrees on the 10 bits they share, and re-widening never overshoots the original.
fn narrowing_shift(bit_depth: BitDepth) -> u32 {
    16 - u32::from(bit_depth.bits())
}

/// Maps an interleaved 16-bit buffer of `n` pixels, `stride` samples apart with `R, G, B` first, to
/// identity GBR planes narrowed to the coded depth.
fn rgb16_to_gbr_planes(px: &[u16], n: usize, stride: usize, shift: u32) -> [Vec<u16>; 3] {
    let mut g = vec![0u16; n];
    let mut b = vec![0u16; n];
    let mut r = vec![0u16; n];
    for i in 0..n {
        r[i] = px[i * stride] >> shift;
        g[i] = px[i * stride + 1] >> shift;
        b[i] = px[i * stride + 2] >> shift;
    }
    [g, b, r]
}

/// Maps an interleaved 16-bit buffer to `Y/Cb/Cr` planes through `matrix`, narrowing to the coded
/// depth **first** — `matrix` is built at that depth and expects its inputs there.
fn rgb16_to_ycbcr_planes(
    px: &[u16],
    n: usize,
    stride: usize,
    shift: u32,
    matrix: RgbToYcbcr,
) -> [Vec<u16>; 3] {
    let mut y = vec![0u16; n];
    let mut cb = vec![0u16; n];
    let mut cr = vec![0u16; n];
    for i in 0..n {
        let (yy, u, v) = matrix.from_rgb(
            px[i * stride] >> shift,
            px[i * stride + 1] >> shift,
            px[i * stride + 2] >> shift,
        );
        y[i] = yy;
        cb[i] = u;
        cr[i] = v;
    }
    [y, cb, cr]
}

/// Three planes of 10-, 12-, or 16-bit samples, row-major, in AV1's `Y/U/V` order.
///
/// The plane conventions are [`Planar8`](crate::Planar8)'s exactly: identity matrix coefficients
/// carry RGB as **Y = G, U = B, V = R**, any other matrix carries `Y, Cb, Cr`, and the buffer's
/// [`ChromaSubsampling`] gives each plane its geometry — including
/// [`Cs400`](ChromaSubsampling::Cs400), which has a luma plane and two empty chroma planes.
///
/// The samples are **left-justified at the coded depth**: a 10-bit plane holds values in
/// `0..=1023`, not 16-bit values to be shifted later. Narrowing a wider source is the caller's
/// decision, made where the loss is visible, and rejected here rather than silently truncated.
#[derive(Debug, Clone)]
pub struct Planar16 {
    width: u32,
    height: u32,
    subsampling: ChromaSubsampling,
    bit_depth: BitDepth,
    planes: [Vec<u16>; 3],
}

impl Planar16 {
    /// Builds a 4:4:4 buffer from three `width * height` planes at `bit_depth`.
    ///
    /// # Errors
    ///
    /// As [`Planar16::from_planes_subsampled`].
    pub fn from_planes(
        width: u32,
        height: u32,
        bit_depth: BitDepth,
        planes: [Vec<u16>; 3],
    ) -> Result<Self> {
        Self::from_planes_subsampled(width, height, ChromaSubsampling::Cs444, bit_depth, planes)
    }

    /// Builds a buffer from three planes whose chroma is subsampled by `subsampling`, at
    /// `bit_depth`.
    ///
    /// Plane 0 must be `width * height` samples and planes 1 and 2 must each match
    /// [`ChromaSubsampling::chroma_dimensions`], exactly as
    /// [`Planar8::from_planes_subsampled`](crate::Planar8::from_planes_subsampled) requires.
    ///
    /// # Examples
    ///
    /// ```
    /// use gamut_color::{BitDepth, Planar16};
    ///
    /// let p = Planar16::from_planes(2, 1, BitDepth::Ten, [vec![1023, 0], vec![512, 512], vec![0, 1023]])?;
    /// assert_eq!(p.bit_depth(), BitDepth::Ten);
    /// assert_eq!(p.plane(0), &[1023u16, 0]);
    ///
    /// // A sample the coded depth cannot represent is a construction error, not a silent clamp.
    /// assert!(Planar16::from_planes(1, 1, BitDepth::Ten, [vec![1024], vec![0], vec![0]]).is_err());
    /// # Ok::<(), gamut_core::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if any plane's length does not match its own dimensions, if
    /// `width * height` overflows `usize`, or if any sample exceeds
    /// [`BitDepth::max_value`] — a plane that does not fit the depth it claims would be coded as a
    /// different image than the caller handed over.
    pub fn from_planes_subsampled(
        width: u32,
        height: u32,
        subsampling: ChromaSubsampling,
        bit_depth: BitDepth,
        planes: [Vec<u16>; 3],
    ) -> Result<Self> {
        let n = Dimensions { width, height }.num_pixels().ok_or_else(|| {
            Error::invalid_input(env!("CARGO_PKG_NAME"), "image dimensions overflow usize")
        })?;
        if planes[0].len() != n {
            return Err(Error::invalid_input(
                env!("CARGO_PKG_NAME"),
                "luma plane length != width * height",
            ));
        }
        let (cw, ch) = subsampling.chroma_dimensions(width, height);
        // Checked per plane, not as a pair: a length mismatch confined to U or to V must not be
        // masked by the other being right.
        let chroma_len = cw as usize * ch as usize;
        if planes[1].len() != chroma_len {
            return Err(Error::invalid_input(
                env!("CARGO_PKG_NAME"),
                "u plane length != chroma dimensions",
            ));
        }
        if planes[2].len() != chroma_len {
            return Err(Error::invalid_input(
                env!("CARGO_PKG_NAME"),
                "v plane length != chroma dimensions",
            ));
        }
        let max = bit_depth.max_value();
        if planes.iter().any(|p| p.iter().any(|&s| s > max)) {
            return Err(Error::invalid_input(
                env!("CARGO_PKG_NAME"),
                "sample exceeds the buffer's bit depth",
            ));
        }
        Ok(Self {
            width,
            height,
            subsampling,
            bit_depth,
            planes,
        })
    }

    /// Maps an interleaved 16-bit RGB image to 4:4:4 identity GBR planes at `bit_depth`.
    ///
    /// The samples arrive on [`gamut_core`]'s canonical full 16-bit scale and are narrowed by
    /// **truncation** — `sample >> (16 - bit_depth)`. The consequence is worth stating plainly: a
    /// lossless encode of the result is bit-exact *at the coded depth*, not to the 16-bit input.
    /// Only [`BitDepth::Sixteen`] keeps every bit.
    ///
    /// # Examples
    ///
    /// ```
    /// use gamut_color::{BitDepth, Planar16};
    /// use gamut_core::{Dimensions, ImageRef, Rgb16};
    ///
    /// let rgb = [0xFFFFu16, 0x8000, 0x0001];
    /// let img = ImageRef::<Rgb16>::new(&rgb, Dimensions::new(1, 1)?)?;
    /// let p = Planar16::from_rgb16_identity_view(img, BitDepth::Twelve);
    /// // GBR order, each sample's top 12 bits.
    /// assert_eq!((p.plane(0)[0], p.plane(1)[0], p.plane(2)[0]), (0x800, 0x000, 0xFFF));
    /// # Ok::<(), gamut_core::Error>(())
    /// ```
    #[must_use]
    pub fn from_rgb16_identity_view(img: ImageRef<'_, Rgb16>, bit_depth: BitDepth) -> Self {
        let (width, height) = (img.width(), img.height());
        let n = width as usize * height as usize;
        Self {
            width,
            height,
            subsampling: ChromaSubsampling::Cs444,
            bit_depth,
            planes: rgb16_to_gbr_planes(img.as_samples(), n, 3, narrowing_shift(bit_depth)),
        }
    }

    /// Maps an interleaved 16-bit RGB image to 4:4:4 `Y/Cb/Cr` planes through `matrix`, narrowed to
    /// `bit_depth` exactly as [`from_rgb16_identity_view`](Self::from_rgb16_identity_view) narrows.
    ///
    /// `matrix` must be built at `bit_depth`: the narrowing happens *before* the transform, so the
    /// matrix sees samples already on the coded scale.
    #[must_use]
    pub fn from_rgb16_matrix_view(
        img: ImageRef<'_, Rgb16>,
        matrix: RgbToYcbcr,
        bit_depth: BitDepth,
    ) -> Self {
        let (width, height) = (img.width(), img.height());
        let n = width as usize * height as usize;
        Self {
            width,
            height,
            subsampling: ChromaSubsampling::Cs444,
            bit_depth,
            planes: rgb16_to_ycbcr_planes(
                img.as_samples(),
                n,
                3,
                narrowing_shift(bit_depth),
                matrix,
            ),
        }
    }

    /// The colour channels of an interleaved 16-bit RGBA image as 4:4:4 identity GBR planes,
    /// ignoring alpha — [`from_rgb16_identity_view`](Self::from_rgb16_identity_view) for a
    /// four-channel source.
    #[must_use]
    pub fn from_rgba16_identity_view(img: ImageRef<'_, Rgba16>, bit_depth: BitDepth) -> Self {
        let (width, height) = (img.width(), img.height());
        let n = width as usize * height as usize;
        Self {
            width,
            height,
            subsampling: ChromaSubsampling::Cs444,
            bit_depth,
            planes: rgb16_to_gbr_planes(img.as_samples(), n, 4, narrowing_shift(bit_depth)),
        }
    }

    /// The colour channels of an interleaved 16-bit RGBA image as 4:4:4 `Y/Cb/Cr` planes through
    /// `matrix`, ignoring alpha.
    #[must_use]
    pub fn from_rgba16_matrix_view(
        img: ImageRef<'_, Rgba16>,
        matrix: RgbToYcbcr,
        bit_depth: BitDepth,
    ) -> Self {
        let (width, height) = (img.width(), img.height());
        let n = width as usize * height as usize;
        Self {
            width,
            height,
            subsampling: ChromaSubsampling::Cs444,
            bit_depth,
            planes: rgb16_to_ycbcr_planes(
                img.as_samples(),
                n,
                4,
                narrowing_shift(bit_depth),
                matrix,
            ),
        }
    }

    /// The **alpha** channel of an interleaved 16-bit RGBA image as monochrome planes at
    /// `bit_depth` — one luma plane carrying the narrowed alpha, and no chroma.
    ///
    /// Alpha is opacity, not colour: it goes through no matrix and no range scaling, exactly as
    /// [`Planar8::from_rgba8_alpha_view`](crate::Planar8::from_rgba8_alpha_view) carries the 8-bit
    /// case.
    #[must_use]
    pub fn from_rgba16_alpha_view(img: ImageRef<'_, Rgba16>, bit_depth: BitDepth) -> Self {
        let shift = narrowing_shift(bit_depth);
        Self {
            width: img.width(),
            height: img.height(),
            subsampling: ChromaSubsampling::Cs400,
            bit_depth,
            planes: [
                img.as_samples()
                    .iter()
                    .skip(3)
                    .step_by(4)
                    .map(|&a| a >> shift)
                    .collect(),
                Vec::new(),
                Vec::new(),
            ],
        }
    }

    /// Image width in samples.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Image height in samples.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The chroma subsampling of the coded planes.
    #[must_use]
    pub fn subsampling(&self) -> ChromaSubsampling {
        self.subsampling
    }

    /// The depth the samples are coded at.
    #[must_use]
    pub fn bit_depth(&self) -> BitDepth {
        self.bit_depth
    }

    /// The dimensions of plane `index`, as [`Planar8::plane_dimensions`](crate::Planar8::plane_dimensions).
    ///
    /// # Panics
    ///
    /// Panics if `index >= 3`.
    #[must_use]
    pub fn plane_dimensions(&self, index: usize) -> (u32, u32) {
        match index {
            0 => (self.width, self.height),
            1 | 2 => self.subsampling.chroma_dimensions(self.width, self.height),
            _ => panic!("plane index {index} out of range (0..3)"),
        }
    }

    /// The row-major samples of plane `index` (`0 = Y/G, 1 = U/B, 2 = V/R`).
    ///
    /// # Panics
    ///
    /// Panics if `index >= 3`, like slice indexing.
    #[must_use]
    pub fn plane(&self, index: usize) -> &[u16] {
        &self.planes[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn planes(bits: BitDepth, luma: Vec<u16>) -> Result<Planar16> {
        Planar16::from_planes(3, 2, bits, [luma, vec![0; 6], vec![0; 6]])
    }

    #[test]
    fn a_buffer_reports_the_geometry_and_depth_it_was_built_with() {
        let p = planes(BitDepth::Twelve, (0..6).map(|i| i * 500).collect()).unwrap();
        assert_eq!((p.width(), p.height()), (3, 2));
        assert_eq!(p.bit_depth(), BitDepth::Twelve);
        assert_eq!(p.subsampling(), ChromaSubsampling::Cs444);
        assert_eq!(p.plane(0), &[0u16, 500, 1000, 1500, 2000, 2500]);
        assert_eq!(p.plane_dimensions(0), (3, 2));
        assert_eq!(p.plane_dimensions(2), (3, 2));
    }

    #[test]
    fn samples_are_validated_against_the_claimed_depth() {
        // 1023 fits 10 bits, 1024 does not — the boundary, not an arbitrary large value.
        assert!(planes(BitDepth::Ten, vec![1023; 6]).is_ok());
        let err = planes(BitDepth::Ten, vec![0, 0, 1024, 0, 0, 0]).expect_err("rejected");
        assert_eq!(
            err.static_message(),
            Some("sample exceeds the buffer's bit depth")
        );
        // The same value is fine at 12 bits, so the check reads the buffer's own depth.
        assert!(planes(BitDepth::Twelve, vec![1024; 6]).is_ok());
        // Chroma is checked too, not just luma.
        assert!(
            Planar16::from_planes(1, 1, BitDepth::Ten, [vec![0], vec![0], vec![2000]]).is_err()
        );
    }

    #[test]
    fn monochrome_has_a_luma_plane_and_no_chroma() {
        let p = Planar16::from_planes_subsampled(
            2,
            2,
            ChromaSubsampling::Cs400,
            BitDepth::Ten,
            [vec![1, 2, 3, 4], Vec::new(), Vec::new()],
        )
        .unwrap();
        assert_eq!(p.plane_dimensions(1), (0, 0));
        assert!(p.plane(1).is_empty());
    }

    #[test]
    fn plane_lengths_are_checked_per_plane() {
        let bad_luma =
            Planar16::from_planes(2, 2, BitDepth::Ten, [vec![0; 3], vec![0; 4], vec![0; 4]]);
        assert_eq!(
            bad_luma.unwrap_err().static_message(),
            Some("luma plane length != width * height")
        );
        let bad_u =
            Planar16::from_planes(2, 2, BitDepth::Ten, [vec![0; 4], vec![0; 3], vec![0; 4]]);
        assert_eq!(
            bad_u.unwrap_err().static_message(),
            Some("u plane length != chroma dimensions")
        );
        let bad_v =
            Planar16::from_planes(2, 2, BitDepth::Ten, [vec![0; 4], vec![0; 4], vec![0; 3]]);
        assert_eq!(
            bad_v.unwrap_err().static_message(),
            Some("v plane length != chroma dimensions")
        );
    }

    /// A 3x2 RGBA16 image whose four channels are pairwise distinct at every pixel, with values
    /// chosen so the discarded low bits differ from the kept high ones.
    ///
    /// 3x2 rather than a square: the pixel count is `width * height`, and for 2x2 that is
    /// indistinguishable from `width + height`.
    fn rgba16_3x2() -> Vec<u16> {
        let mut px = Vec::new();
        for i in 0..6u16 {
            px.extend_from_slice(&[0xF000 | i, 0x8000 | (i << 4), 0x0FFF ^ i, 0xFFFF - (i << 8)]);
        }
        px
    }

    #[test]
    fn sixteen_bit_rgb_is_narrowed_by_truncation() {
        // The contract: the coded sample is the *top* `bit_depth` bits of the input, and the low
        // bits are dropped rather than rounded — `0xFFFF` stays at the top of the range instead of
        // rounding out of it, and `0x0FFF` keeps only what fits above the shift.
        let rgb = [0xFFFFu16, 0x8000, 0x0FFF];
        let img = ImageRef::<Rgb16>::new(&rgb, Dimensions::new(1, 1).unwrap()).unwrap();
        for (bits, want) in [
            (BitDepth::Ten, (0x200u16, 0x03F, 0x3FF)),
            (BitDepth::Twelve, (0x800, 0x0FF, 0xFFF)),
            // Sixteen keeps every bit, so the shift is the identity.
            (BitDepth::Sixteen, (0x8000, 0x0FFF, 0xFFFF)),
        ] {
            // GBR order: plane0 = G, plane1 = B, plane2 = R.
            let p = Planar16::from_rgb16_identity_view(img, bits);
            assert_eq!(
                (p.plane(0)[0], p.plane(1)[0], p.plane(2)[0]),
                want,
                "{bits:?}"
            );
            assert_eq!(p.bit_depth(), bits);
            // Every narrowed sample fits the claimed depth, which is what `from_planes_subsampled`
            // would otherwise reject.
            assert!(p.plane(2)[0] <= bits.max_value());
        }
    }

    /// A 3x2 RGB16 image whose three channels are pairwise distinct at every pixel, with low bits
    /// that survive no shift, so a narrowing in the wrong direction or from the wrong channel
    /// cannot land on the right value.
    fn rgb16_3x2() -> Vec<u16> {
        let mut px = Vec::new();
        for i in 0..6u16 {
            px.extend_from_slice(&[0xF00D ^ (i << 3), 0x81C7 | i, 0x0FF1 + (i << 6)]);
        }
        px
    }

    #[test]
    fn sixteen_bit_matrix_planes_narrow_before_the_transform() {
        // `RgbToYcbcr` is built at the coded depth, so the narrowing has to happen *first* — the
        // matrix must see samples already on the coded scale. The reference below applies the shift
        // itself and then the same transform, so a narrowing done in the wrong direction, from the
        // wrong channel, or at the wrong pixel gives a different plane.
        let px = rgb16_3x2();
        let dims = Dimensions::new(3, 2).unwrap();
        let img = ImageRef::<Rgb16>::new(&px, dims).unwrap();
        let matrix = RgbToYcbcr::new(
            crate::MatrixCoefficients::Bt709,
            crate::ColorRange::Full,
            BitDepth::Twelve,
        )
        .unwrap();
        let p = Planar16::from_rgb16_matrix_view(img, matrix, BitDepth::Twelve);

        let mut want: [Vec<u16>; 3] = [Vec::new(), Vec::new(), Vec::new()];
        for pixel in px.as_chunks::<3>().0 {
            let (y, cb, cr) = matrix.from_rgb(pixel[0] >> 4, pixel[1] >> 4, pixel[2] >> 4);
            want[0].push(y);
            want[1].push(cb);
            want[2].push(cr);
        }
        for (i, w) in want.iter().enumerate() {
            assert_eq!(p.plane(i), &w[..], "plane {i}");
            assert_eq!(p.plane(i).len(), 6, "plane {i} covers every pixel");
        }
        // The three planes differ, so a mapping that read one channel three times could not satisfy
        // the comparison above by coincidence.
        assert_ne!(p.plane(0), p.plane(1));
        assert_ne!(p.plane(1), p.plane(2));

        // The four-channel source gives the same colour planes for the same colour values — the
        // property that makes an RGBA colour item identical to the RGB one through the matrix path
        // as well as the identity one.
        let rgba: Vec<u16> = px
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|c| [c[0], c[1], c[2], 0xFFFF])
            .collect();
        let q = Planar16::from_rgba16_matrix_view(
            ImageRef::<Rgba16>::new(&rgba, dims).unwrap(),
            matrix,
            BitDepth::Twelve,
        );
        for i in 0..3 {
            assert_eq!(q.plane(i), p.plane(i), "rgba plane {i}");
        }
    }

    #[test]
    fn rgba16_colour_planes_ignore_alpha_and_alpha_is_its_own_plane() {
        let px = rgba16_3x2();
        let img = ImageRef::<Rgba16>::new(&px, Dimensions::new(3, 2).unwrap()).unwrap();
        let colour = Planar16::from_rgba16_identity_view(img, BitDepth::Twelve);
        assert_eq!(colour.subsampling(), ChromaSubsampling::Cs444);
        // Same mapping the three-channel constructor gives for the same colour values, which is
        // what makes an RGBA colour item identical to the RGB one.
        let rgb: Vec<u16> = px
            .as_chunks::<4>()
            .0
            .iter()
            .flat_map(|c| c[..3].to_vec())
            .collect();
        let from_rgb = Planar16::from_rgb16_identity_view(
            ImageRef::<Rgb16>::new(&rgb, Dimensions::new(3, 2).unwrap()).unwrap(),
            BitDepth::Twelve,
        );
        for i in 0..3 {
            assert_eq!(colour.plane(i), from_rgb.plane(i), "plane {i}");
            assert_eq!(colour.plane(i).len(), 6, "plane {i} covers every pixel");
        }

        let alpha = Planar16::from_rgba16_alpha_view(img, BitDepth::Twelve);
        assert_eq!(alpha.subsampling(), ChromaSubsampling::Cs400);
        // The fourth channel of each pixel, narrowed — not the first, and not every fourth sample
        // from the start.
        let want: Vec<u16> = (0..6u16).map(|i| (0xFFFF - (i << 8)) >> 4).collect();
        assert_eq!(alpha.plane(0), &want[..]);
        assert!(alpha.plane(1).is_empty());
        assert_eq!(alpha.plane_dimensions(1), (0, 0));
    }

    #[test]
    #[should_panic(expected = "plane index 3 out of range")]
    fn plane_dimensions_rejects_an_out_of_range_index() {
        let p = planes(BitDepth::Ten, vec![0; 6]).unwrap();
        let _ = p.plane_dimensions(3);
    }
}
