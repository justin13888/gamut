//! Planar 8-bit image buffers and the RGB → plane mappings: identity (`mc = 0`) and the CICP
//! luma–chroma matrices.

use gamut_core::{Dimensions, Error, ImageRef, Result, Rgb8};

use crate::format::ChromaSubsampling;
use crate::ycbcr::RgbToYcbcr;

/// Maps an interleaved RGB buffer (`n` pixels) to identity GBR planes (`Y=G, U=B, V=R`).
fn rgb_to_gbr_planes(rgb: &[u8], n: usize) -> [Vec<u8>; 3] {
    let mut g = vec![0u8; n];
    let mut b = vec![0u8; n];
    let mut r = vec![0u8; n];
    for i in 0..n {
        r[i] = rgb[i * 3];
        g[i] = rgb[i * 3 + 1];
        b[i] = rgb[i * 3 + 2];
    }
    [g, b, r]
}

/// Maps an interleaved RGB buffer (`n` pixels) to `Y/Cb/Cr` planes through `matrix`.
fn rgb_to_ycbcr_planes(rgb: &[u8], n: usize, matrix: RgbToYcbcr) -> [Vec<u8>; 3] {
    let mut y = vec![0u8; n];
    let mut cb = vec![0u8; n];
    let mut cr = vec![0u8; n];
    for i in 0..n {
        let (yy, u, v) = matrix.from_rgb(
            u16::from(rgb[i * 3]),
            u16::from(rgb[i * 3 + 1]),
            u16::from(rgb[i * 3 + 2]),
        );
        // `matrix` is built at `BitDepth::Eight`, so every output is already in `0..=255`.
        y[i] = yy as u8;
        cb[i] = u as u8;
        cr[i] = v as u8;
    }
    [y, cb, cr]
}

/// Three full-resolution (4:4:4) 8-bit planes, each `width * height` samples, row-major.
///
/// For identity matrix coefficients (CICP `mc = 0`) AV1 carries RGB directly with the plane order
/// **Y = G, U = B, V = R** ("GBR"); [`Planar8::from_rgb8_identity`] performs that mapping and
/// [`Planar8::to_rgb8_identity`] reverses it. Keeping the convention in one place means the
/// end-to-end round-trip (decode via `avifdec`) is the single source of truth for its correctness.
///
/// For any other matrix the planes are **Y, Cb, Cr** in AV1's `Y/U/V` order;
/// [`Planar8::from_rgb8_matrix`] applies the H.273 transform of [`RgbToYcbcr`].
///
/// The buffer carries its [`ChromaSubsampling`], so the chroma planes need not be full resolution.
/// Every RGB constructor produces [`Cs444`](ChromaSubsampling::Cs444); subsampled buffers are built
/// with [`from_planes_subsampled`](Self::from_planes_subsampled). The *format* is stored rather than
/// three plane sizes, so there is no inconsistent state to keep in sync —
/// [`plane_dimensions`](Self::plane_dimensions) derives each plane's geometry from
/// `(width, height, subsampling)`.
#[derive(Debug, Clone)]
pub struct Planar8 {
    width: u32,
    height: u32,
    subsampling: ChromaSubsampling,
    planes: [Vec<u8>; 3],
}

impl Planar8 {
    /// Maps an interleaved 8-bit RGB buffer to identity planes (`Y=G, U=B, V=R`).
    ///
    /// # Examples
    ///
    /// ```
    /// use gamut_color::Planar8;
    /// let rgb = [10, 20, 30, 40, 50, 60]; // two RGB pixels
    /// let planes = Planar8::from_rgb8_identity(&rgb, 2, 1).expect("valid length");
    /// assert_eq!(planes.plane(0), &[20u8, 50]); // Y carries G
    /// assert_eq!(planes.to_rgb8_identity(), rgb); // round-trips
    /// ```
    ///
    /// Zero dimensions are allowed (an empty buffer): rejecting empty images is the encoder's
    /// decision, made at its own boundary.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `rgb.len() != width * height * 3`, or if that product
    /// overflows `usize`.
    pub fn from_rgb8_identity(rgb: &[u8], width: u32, height: u32) -> Result<Self> {
        let n = Dimensions { width, height }.num_pixels().ok_or_else(|| {
            Error::invalid_input(env!("CARGO_PKG_NAME"), "image dimensions overflow usize")
        })?;
        if n.checked_mul(3) != Some(rgb.len()) {
            return Err(Error::invalid_input(
                env!("CARGO_PKG_NAME"),
                "rgb buffer length != width * height * 3",
            ));
        }
        Ok(Self {
            width,
            height,
            subsampling: ChromaSubsampling::Cs444,
            planes: rgb_to_gbr_planes(rgb, n),
        })
    }

    /// Like [`Planar8::from_rgb8_identity`] but takes a pre-validated [`ImageRef`], so it is
    /// infallible — the view already guarantees `rgb.len() == width * height * 3`. This is the
    /// boundary an encoder uses to turn a typed RGB image into AV1 identity planes.
    #[must_use]
    pub fn from_rgb8_identity_view(img: ImageRef<'_, Rgb8>) -> Self {
        let (width, height) = (img.width(), img.height());
        // No overflow check needed: `ImageRef` already validated that width * height * 3 fits
        // `usize` (it equals the sample slice's length).
        let n = width as usize * height as usize;
        Self {
            width,
            height,
            subsampling: ChromaSubsampling::Cs444,
            planes: rgb_to_gbr_planes(img.as_samples(), n),
        }
    }

    /// Maps an interleaved 8-bit RGB buffer to 4:4:4 `Y/Cb/Cr` planes through `matrix`.
    ///
    /// The counterpart of [`Planar8::from_rgb8_identity`] for every CICP matrix that *does* apply a
    /// luma–chroma transform; build `matrix` once with
    /// [`RgbToYcbcr::new`](crate::RgbToYcbcr::new) at [`BitDepth::Eight`](crate::BitDepth::Eight),
    /// which is where an unsupported matrix is rejected.
    ///
    /// # Examples
    ///
    /// ```
    /// use gamut_color::{BitDepth, ColorRange, MatrixCoefficients, Planar8, RgbToYcbcr};
    ///
    /// let m = RgbToYcbcr::new(MatrixCoefficients::Bt709, ColorRange::Full, BitDepth::Eight)?;
    /// let rgb = [255, 255, 255, 0, 0, 0]; // white then black
    /// let planes = Planar8::from_rgb8_matrix(&rgb, 2, 1, m)?;
    /// assert_eq!(planes.plane(0), &[255u8, 0]); // luma
    /// assert_eq!(planes.plane(1), &[128u8, 128]); // neutral chroma
    /// # Ok::<(), gamut_core::Error>(())
    /// ```
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `rgb.len() != width * height * 3`, or if that product
    /// overflows `usize`.
    pub fn from_rgb8_matrix(
        rgb: &[u8],
        width: u32,
        height: u32,
        matrix: RgbToYcbcr,
    ) -> Result<Self> {
        let n = Dimensions { width, height }.num_pixels().ok_or_else(|| {
            Error::invalid_input(env!("CARGO_PKG_NAME"), "image dimensions overflow usize")
        })?;
        if n.checked_mul(3) != Some(rgb.len()) {
            return Err(Error::invalid_input(
                env!("CARGO_PKG_NAME"),
                "rgb buffer length != width * height * 3",
            ));
        }
        Ok(Self {
            width,
            height,
            subsampling: ChromaSubsampling::Cs444,
            planes: rgb_to_ycbcr_planes(rgb, n, matrix),
        })
    }

    /// Like [`Planar8::from_rgb8_matrix`] but takes a pre-validated [`ImageRef`], so it is
    /// infallible — the view already guarantees `rgb.len() == width * height * 3`. This is the
    /// boundary an encoder uses to turn a typed RGB image into AV1 YCbCr planes.
    #[must_use]
    pub fn from_rgb8_matrix_view(img: ImageRef<'_, Rgb8>, matrix: RgbToYcbcr) -> Self {
        let (width, height) = (img.width(), img.height());
        // No overflow check needed: `ImageRef` already validated that width * height * 3 fits
        // `usize` (it equals the sample slice's length).
        let n = width as usize * height as usize;
        Self {
            width,
            height,
            subsampling: ChromaSubsampling::Cs444,
            planes: rgb_to_ycbcr_planes(img.as_samples(), n, matrix),
        }
    }

    /// Builds a `Planar8` directly from three `width * height` planes (`Y/U/V`, already in the
    /// identity GBR order). Used by the encoder to wrap a horizontally-downscaled source for superres.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if any plane's length is not `width * height`, or if that
    /// product overflows `usize`.
    pub fn from_planes(width: u32, height: u32, planes: [Vec<u8>; 3]) -> Result<Self> {
        Self::from_planes_subsampled(width, height, ChromaSubsampling::Cs444, planes)
    }

    /// Builds a `Planar8` from three planes whose chroma is subsampled by `subsampling`.
    ///
    /// Plane 0 must be `width * height` samples; planes 1 and 2 must each match
    /// [`ChromaSubsampling::chroma_dimensions`] for `(width, height)` — **ceiling** division on the
    /// subsampled axes, so a 5×3 image in 4:2:0 needs 3×2 chroma planes.
    /// [`ChromaSubsampling::Cs400`] has no chroma, so both chroma planes must be empty.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if any plane's length does not match its own dimensions, or
    /// if `width * height` overflows `usize`.
    pub fn from_planes_subsampled(
        width: u32,
        height: u32,
        subsampling: ChromaSubsampling,
        planes: [Vec<u8>; 3],
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
        Ok(Self {
            width,
            height,
            subsampling,
            planes,
        })
    }

    /// Reverses [`Planar8::from_rgb8_identity`], producing an interleaved 8-bit RGB buffer.
    ///
    /// For a subsampled buffer the chroma planes are expanded by **nearest-neighbour replication**
    /// (each output sample reads `plane[(y >> ss_y) * cw + (x >> ss_x)]`). Identity coefficients
    /// require 4:4:4 in a conformant AV1 stream (§6.4.2), so this path exists to keep the accessor
    /// total, not as a resampling filter. [`ChromaSubsampling::Cs400`] has no chroma planes and
    /// yields gray (`R = G = B = Y`).
    #[must_use]
    pub fn to_rgb8_identity(&self) -> Vec<u8> {
        let n = self.width as usize * self.height as usize;
        let (g, b, r) = (&self.planes[0], &self.planes[1], &self.planes[2]);
        let mut out = vec![0u8; n * 3];
        if self.subsampling == ChromaSubsampling::Cs444 {
            // Fast path: every plane shares the luma index, so this is a straight de-interleave.
            for i in 0..n {
                out[i * 3] = r[i];
                out[i * 3 + 1] = g[i];
                out[i * 3 + 2] = b[i];
            }
            return out;
        }
        if self.subsampling == ChromaSubsampling::Cs400 {
            for i in 0..n {
                out[i * 3] = g[i];
                out[i * 3 + 1] = g[i];
                out[i * 3 + 2] = g[i];
            }
            return out;
        }
        let (sx, sy) = self.subsampling.subsampling();
        let (cw, _) = self.subsampling.chroma_dimensions(self.width, self.height);
        let (w, cw) = (self.width as usize, cw as usize);
        for y in 0..self.height as usize {
            // In bounds without a runtime check: for `x < width`, `x >> sx` is at most
            // `(width - 1) >> sx`, which is strictly less than `ceil(width / (1 << sx))` = `cw`.
            // The same argument bounds the row against the chroma height.
            let crow = (y >> sy) * cw;
            for x in 0..w {
                let i = y * w + x;
                let ci = crow + (x >> sx);
                out[i * 3] = r[ci];
                out[i * 3 + 1] = g[i];
                out[i * 3 + 2] = b[ci];
            }
        }
        out
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

    /// The dimensions of plane `index`: `(width, height)` for luma, and
    /// [`ChromaSubsampling::chroma_dimensions`] for the two chroma planes.
    ///
    /// # Panics
    ///
    /// Panics if `index >= 3`, matching [`plane`](Self::plane).
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
    /// The slice is `w * h` samples for the plane's own
    /// [`plane_dimensions`](Self::plane_dimensions), which for a subsampled buffer is smaller than
    /// the luma plane.
    ///
    /// # Panics
    ///
    /// Panics if `index >= 3`, like slice indexing — an out-of-range plane index is a
    /// programmer error, not a data error.
    #[must_use]
    pub fn plane(&self, index: usize) -> &[u8] {
        &self.planes[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_identity_round_trips_with_gbr_order() {
        // One pixel (R=10, G=20, B=30) pins the plane order directly: Y=G=20, U=B=30, V=R=10. The
        // round-trip alone is order-agnostic (a consistent from/to swap would still pass), and the
        // GBR convention is the module's whole point.
        let p = Planar8::from_rgb8_identity(&[10, 20, 30], 1, 1).unwrap();
        assert_eq!(p.plane(0), &[20]);
        assert_eq!(p.plane(1), &[30]);
        assert_eq!(p.plane(2), &[10]);

        let rgb: Vec<u8> = (0..=200u8).cycle().take(2 * 3 * 3).collect(); // 3x2 image
        let p = Planar8::from_rgb8_identity(&rgb, 3, 2).unwrap();
        assert_eq!(p.width(), 3);
        assert_eq!(p.height(), 2);
        assert_eq!(p.to_rgb8_identity(), rgb);
    }

    #[test]
    fn wrong_length_errors() {
        assert!(Planar8::from_rgb8_identity(&[0, 1, 2, 3], 1, 1).is_err());
    }

    #[test]
    fn from_planes_validates_and_wraps() {
        // 3x2 ⇒ n = 6. Three distinct planes, each length 6.
        let g: Vec<u8> = (0..6).collect();
        let b: Vec<u8> = (10..16).collect();
        let r: Vec<u8> = (20..26).collect();
        let p = Planar8::from_planes(3, 2, [g.clone(), b.clone(), r.clone()]).unwrap();
        assert_eq!((p.width(), p.height()), (3, 2));
        assert_eq!(p.plane(0), &g[..]);
        assert_eq!(p.plane(1), &b[..]);
        assert_eq!(p.plane(2), &r[..]);
        // The valid case above only passes when `n == 3 * 2`: a mutated `width * height` (3 + 2 = 5,
        // 3 / 2 = 1) or an inverted `!=` length check would reject these correctly-sized planes.
        assert!(Planar8::from_planes(3, 2, [vec![0; 6], vec![0; 6], vec![0; 5]]).is_err());
        assert!(Planar8::from_planes(3, 2, [vec![0; 5], vec![0; 6], vec![0; 6]]).is_err());
    }

    #[test]
    fn rejects_overflowing_dimensions() {
        // Near-max dimensions must yield Err, not an overflow panic (debug) or a wrapped length
        // check (32-bit release): width * height * 3 exceeds usize even on 64-bit targets.
        assert!(Planar8::from_rgb8_identity(&[], u32::MAX, u32::MAX).is_err());
        assert!(Planar8::from_planes(u32::MAX, u32::MAX, [vec![], vec![], vec![]]).is_err());
    }

    #[test]
    fn matrix_ctor_writes_y_cb_cr_in_plane_order() {
        use crate::cicp::{ColorRange, MatrixCoefficients};
        use crate::format::BitDepth;
        use crate::ycbcr::RgbToYcbcr;

        // Pure red under BT.709 full range: Y = 54, Cb = 99, Cr = 255 (pinned in `ycbcr`).
        // Asserting per plane pins the Y/Cb/Cr order — the opposite of the identity path's GBR.
        let m =
            RgbToYcbcr::new(MatrixCoefficients::Bt709, ColorRange::Full, BitDepth::Eight).unwrap();
        let p = Planar8::from_rgb8_matrix(&[255, 0, 0], 1, 1, m).unwrap();
        assert_eq!(
            (p.plane(0), p.plane(1), p.plane(2)),
            (&[54u8][..], &[99u8][..], &[255u8][..])
        );

        // The view constructor must agree sample for sample. Saturated, strongly-coloured pixels,
        // so the luma plane cannot coincidentally equal the green channel the identity path uses.
        let rgb: Vec<u8> = vec![
            255, 0, 0, 0, 255, 0, 0, 0, 255, 10, 200, 30, 250, 5, 60, 128, 128, 128,
        ];
        let from_slice = Planar8::from_rgb8_matrix(&rgb, 3, 2, m).unwrap();
        let view = ImageRef::<Rgb8>::new(&rgb, Dimensions::new(3, 2).unwrap()).unwrap();
        let from_view = Planar8::from_rgb8_matrix_view(view, m);
        for i in 0..3 {
            assert_eq!(from_view.plane(i), from_slice.plane(i));
        }
        // …and the transform really ran: the planes are not the identity mapping.
        let identity = Planar8::from_rgb8_identity(&rgb, 3, 2).unwrap();
        assert_ne!(from_slice.plane(0), identity.plane(0));

        assert!(Planar8::from_rgb8_matrix(&[0, 1, 2, 3], 1, 1, m).is_err());
        assert!(Planar8::from_rgb8_matrix(&[], u32::MAX, u32::MAX, m).is_err());
    }

    #[test]
    fn view_ctor_matches_slice_ctor() {
        let rgb: Vec<u8> = (0..=200u8).cycle().take(3 * 2 * 3).collect(); // 3x2 image
        let from_slice = Planar8::from_rgb8_identity(&rgb, 3, 2).unwrap();
        let view = ImageRef::<Rgb8>::new(&rgb, gamut_core::Dimensions::new(3, 2).unwrap()).unwrap();
        let from_view = Planar8::from_rgb8_identity_view(view);
        assert_eq!((from_view.width(), from_view.height()), (3, 2));
        for i in 0..3 {
            assert_eq!(from_view.plane(i), from_slice.plane(i));
        }
    }

    #[test]
    fn plane_dimensions_follow_the_subsampling() {
        // 4:4:4 from an RGB constructor: every plane is full resolution.
        let p = Planar8::from_rgb8_identity(&[0; 5 * 3 * 3], 5, 3).unwrap();
        assert_eq!(p.subsampling(), ChromaSubsampling::Cs444);
        for i in 0..3 {
            assert_eq!(p.plane_dimensions(i), (5, 3));
        }

        // Odd dimensions are the interesting case: 5x3 in 4:2:0 has 3x2 chroma (ceiling), and in
        // 4:2:2 has 3x3. A mutant that floors either axis produces 2x1 / 2x3 and dies here.
        let p420 = Planar8::from_planes_subsampled(
            5,
            3,
            ChromaSubsampling::Cs420,
            [vec![0; 15], vec![0; 6], vec![0; 6]],
        )
        .unwrap();
        assert_eq!(p420.plane_dimensions(0), (5, 3));
        assert_eq!(p420.plane_dimensions(1), (3, 2));
        assert_eq!(p420.plane_dimensions(2), (3, 2));

        let p422 = Planar8::from_planes_subsampled(
            5,
            3,
            ChromaSubsampling::Cs422,
            [vec![0; 15], vec![0; 9], vec![0; 9]],
        )
        .unwrap();
        assert_eq!(p422.plane_dimensions(1), (3, 3));
    }

    #[test]
    fn from_planes_subsampled_rejects_a_wrong_luma_plane() {
        // 5x3 in 4:2:0 ⇒ luma 15, chroma 3x2 = 6 each.
        let err = Planar8::from_planes_subsampled(
            5,
            3,
            ChromaSubsampling::Cs420,
            [vec![0; 14], vec![0; 6], vec![0; 6]],
        )
        .expect_err("short luma plane");
        // Asserting the specific diagnostic, not `is_err()`: the chroma checks below would reject a
        // great many wrong inputs too, so only the message distinguishes this guard from them.
        assert_eq!(
            err.static_message(),
            Some("luma plane length != width * height")
        );
    }

    #[test]
    fn from_planes_subsampled_rejects_a_wrong_u_plane() {
        let err = Planar8::from_planes_subsampled(
            5,
            3,
            ChromaSubsampling::Cs420,
            [vec![0; 15], vec![0; 15], vec![0; 6]],
        )
        .expect_err("full-resolution u plane");
        assert_eq!(
            err.static_message(),
            Some("u plane length != chroma dimensions")
        );
    }

    #[test]
    fn from_planes_subsampled_rejects_a_wrong_v_plane() {
        // Separate from the U case on purpose: a guard that validated only `planes[1]` would pass
        // the test above and fail only here.
        let err = Planar8::from_planes_subsampled(
            5,
            3,
            ChromaSubsampling::Cs420,
            [vec![0; 15], vec![0; 6], vec![0; 15]],
        )
        .expect_err("full-resolution v plane");
        assert_eq!(
            err.static_message(),
            Some("v plane length != chroma dimensions")
        );
    }

    #[test]
    fn from_planes_is_the_four_four_four_case_of_from_planes_subsampled() {
        let planes = [vec![1u8; 6], vec![2; 6], vec![3; 6]];
        let a = Planar8::from_planes(3, 2, planes.clone()).unwrap();
        let b = Planar8::from_planes_subsampled(3, 2, ChromaSubsampling::Cs444, planes).unwrap();
        assert_eq!(a.subsampling(), ChromaSubsampling::Cs444);
        assert_eq!(a.subsampling(), b.subsampling());
        for i in 0..3 {
            assert_eq!(a.plane(i), b.plane(i));
            assert_eq!(a.plane_dimensions(i), b.plane_dimensions(i));
        }
    }

    #[test]
    fn to_rgb8_identity_replicates_subsampled_chroma() {
        // 3x3 in 4:2:0 ⇒ 2x2 chroma. Distinct chroma values per quadrant pin the replication map
        // and, at the odd right column and bottom row, the edge clamp implied by ceiling division.
        let luma: Vec<u8> = (0..9).collect();
        let u = vec![10u8, 20, 30, 40];
        let v = vec![50u8, 60, 70, 80];
        let p =
            Planar8::from_planes_subsampled(3, 3, ChromaSubsampling::Cs420, [luma, u, v]).unwrap();
        let rgb = p.to_rgb8_identity();
        // Identity order is R=V, G=Y, B=U. Chroma index is (y >> 1) * 2 + (x >> 1).
        let at = |x: usize, y: usize| {
            let i = (y * 3 + x) * 3;
            (rgb[i], rgb[i + 1], rgb[i + 2])
        };
        assert_eq!(at(0, 0), (50, 0, 10));
        assert_eq!(at(1, 0), (50, 1, 10)); // same chroma sample as (0,0)
        assert_eq!(at(2, 0), (60, 2, 20)); // odd right column reads chroma column 1
        assert_eq!(at(0, 2), (70, 6, 30)); // odd bottom row reads chroma row 1
        assert_eq!(at(2, 2), (80, 8, 40));
    }

    #[test]
    fn to_rgb8_identity_is_unchanged_on_the_four_four_four_fast_path() {
        // The general path must agree with the fast path wherever both are defined, so a 4:4:4
        // buffer built through `from_planes_subsampled` round-trips identically.
        let rgb: Vec<u8> = (0..(4 * 3 * 3) as u8).collect();
        let p = Planar8::from_rgb8_identity(&rgb, 4, 3).unwrap();
        assert_eq!(p.to_rgb8_identity(), rgb);
    }

    #[test]
    fn to_rgb8_identity_of_monochrome_is_gray() {
        let p = Planar8::from_planes_subsampled(
            2,
            2,
            ChromaSubsampling::Cs400,
            [vec![7, 8, 9, 10], Vec::new(), Vec::new()],
        )
        .unwrap();
        assert_eq!(
            p.to_rgb8_identity(),
            vec![7, 7, 7, 8, 8, 8, 9, 9, 9, 10, 10, 10]
        );
    }

    #[test]
    #[should_panic(expected = "plane index 3 out of range")]
    fn plane_dimensions_rejects_an_out_of_range_index() {
        let p = Planar8::from_rgb8_identity(&[0; 3], 1, 1).unwrap();
        let _ = p.plane_dimensions(3);
    }
}
