//! Planar 8-bit image buffers and the RGB → plane mappings: identity (`mc = 0`) and the CICP
//! luma–chroma matrices.

use gamut_core::{Dimensions, Error, ImageRef, Result, Rgb8};

use crate::ycbcr_matrix::YcbcrMatrix;

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
fn rgb_to_ycbcr_planes(rgb: &[u8], n: usize, matrix: YcbcrMatrix) -> [Vec<u8>; 3] {
    let mut y = vec![0u8; n];
    let mut cb = vec![0u8; n];
    let mut cr = vec![0u8; n];
    for i in 0..n {
        let (yy, u, v) = matrix.forward(rgb[i * 3], rgb[i * 3 + 1], rgb[i * 3 + 2]);
        y[i] = yy;
        cb[i] = u;
        cr[i] = v;
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
/// [`Planar8::from_rgb8_matrix`] applies the H.273 transform of [`YcbcrMatrix`]. Chroma stays at
/// full resolution either way — subsampled (4:2:0 / 4:2:2) plane geometry is not modelled here yet.
#[derive(Debug, Clone)]
pub struct Planar8 {
    width: u32,
    height: u32,
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
            planes: rgb_to_gbr_planes(img.as_samples(), n),
        }
    }

    /// Maps an interleaved 8-bit RGB buffer to 4:4:4 `Y/Cb/Cr` planes through `matrix`.
    ///
    /// The counterpart of [`Planar8::from_rgb8_identity`] for every CICP matrix that *does* apply a
    /// luma–chroma transform; build `matrix` once with
    /// [`YcbcrMatrix::new`](crate::YcbcrMatrix::new), which is where an unsupported matrix is
    /// rejected.
    ///
    /// # Examples
    ///
    /// ```
    /// use gamut_color::{ColorRange, MatrixCoefficients, Planar8, YcbcrMatrix};
    ///
    /// let m = YcbcrMatrix::new(MatrixCoefficients::Bt709, ColorRange::Full)?;
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
        matrix: YcbcrMatrix,
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
            planes: rgb_to_ycbcr_planes(rgb, n, matrix),
        })
    }

    /// Like [`Planar8::from_rgb8_matrix`] but takes a pre-validated [`ImageRef`], so it is
    /// infallible — the view already guarantees `rgb.len() == width * height * 3`. This is the
    /// boundary an encoder uses to turn a typed RGB image into AV1 YCbCr planes.
    #[must_use]
    pub fn from_rgb8_matrix_view(img: ImageRef<'_, Rgb8>, matrix: YcbcrMatrix) -> Self {
        let (width, height) = (img.width(), img.height());
        // No overflow check needed: `ImageRef` already validated that width * height * 3 fits
        // `usize` (it equals the sample slice's length).
        let n = width as usize * height as usize;
        Self {
            width,
            height,
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
        let n = Dimensions { width, height }.num_pixels().ok_or_else(|| {
            Error::invalid_input(env!("CARGO_PKG_NAME"), "image dimensions overflow usize")
        })?;
        if planes.iter().any(|p| p.len() != n) {
            return Err(Error::invalid_input(
                env!("CARGO_PKG_NAME"),
                "plane length != width * height",
            ));
        }
        Ok(Self {
            width,
            height,
            planes,
        })
    }

    /// Reverses [`Planar8::from_rgb8_identity`], producing an interleaved 8-bit RGB buffer.
    #[must_use]
    pub fn to_rgb8_identity(&self) -> Vec<u8> {
        let n = self.width as usize * self.height as usize;
        let (g, b, r) = (&self.planes[0], &self.planes[1], &self.planes[2]);
        let mut out = vec![0u8; n * 3];
        for i in 0..n {
            out[i * 3] = r[i];
            out[i * 3 + 1] = g[i];
            out[i * 3 + 2] = b[i];
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

    /// The row-major samples of plane `index` (`0 = Y/G, 1 = U/B, 2 = V/R`).
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
        use crate::ycbcr_matrix::YcbcrMatrix;

        // Pure red under BT.709 full range: Y = 54, Cb = 99, Cr = 255 (pinned in `ycbcr_matrix`).
        // Asserting per plane pins the Y/Cb/Cr order — the opposite of the identity path's GBR.
        let m = YcbcrMatrix::new(MatrixCoefficients::Bt709, ColorRange::Full).unwrap();
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
}
