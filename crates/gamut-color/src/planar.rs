//! Planar 8-bit image buffers and the identity (`mc = 0`) RGB ↔ plane mapping.

use gamut_core::{Error, ImageRef, Result, Rgb8};

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

/// Three full-resolution (4:4:4) 8-bit planes, each `width * height` samples, row-major.
///
/// For identity matrix coefficients (CICP `mc = 0`) AV1 carries RGB directly with the plane order
/// **Y = G, U = B, V = R** ("GBR"); [`Planar8::from_rgb8_identity`] performs that mapping and
/// [`Planar8::to_rgb8_identity`] reverses it. Keeping the convention in one place means the
/// end-to-end round-trip (decode via `avifdec`) is the single source of truth for its correctness.
#[derive(Debug, Clone)]
pub struct Planar8 {
    width: u32,
    height: u32,
    planes: [Vec<u8>; 3],
}

impl Planar8 {
    /// Maps an interleaved 8-bit RGB buffer to identity planes (`Y=G, U=B, V=R`).
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `rgb.len() != width * height * 3`.
    pub fn from_rgb8_identity(rgb: &[u8], width: u32, height: u32) -> Result<Self> {
        let n = width as usize * height as usize;
        if rgb.len() != n * 3 {
            return Err(Error::InvalidInput(
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
        let n = width as usize * height as usize;
        Self {
            width,
            height,
            planes: rgb_to_gbr_planes(img.as_samples(), n),
        }
    }

    /// Builds a `Planar8` directly from three `width * height` planes (`Y/U/V`, already in the
    /// identity GBR order). Used by the encoder to wrap a horizontally-downscaled source for superres.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if any plane's length is not `width * height`.
    pub fn from_planes(width: u32, height: u32, planes: [Vec<u8>; 3]) -> Result<Self> {
        let n = width as usize * height as usize;
        if planes.iter().any(|p| p.len() != n) {
            return Err(Error::InvalidInput("plane length != width * height"));
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
    #[must_use]
    pub fn plane(&self, index: usize) -> &[u8] {
        &self.planes[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgb_identity_plane_order() {
        // One pixel (R=10, G=20, B=30): Y=G=20, U=B=30, V=R=10.
        let p = Planar8::from_rgb8_identity(&[10, 20, 30], 1, 1).unwrap();
        assert_eq!(p.plane(0), &[20]);
        assert_eq!(p.plane(1), &[30]);
        assert_eq!(p.plane(2), &[10]);
    }

    #[test]
    fn rgb_roundtrip() {
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
