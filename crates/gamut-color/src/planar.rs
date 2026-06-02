//! Planar 8-bit image buffers and the identity (`mc = 0`) RGB ↔ plane mapping.

use gamut_core::{Error, Result};

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
        let mut g = vec![0u8; n];
        let mut b = vec![0u8; n];
        let mut r = vec![0u8; n];
        for i in 0..n {
            r[i] = rgb[i * 3];
            g[i] = rgb[i * 3 + 1];
            b[i] = rgb[i * 3 + 2];
        }
        Ok(Self {
            width,
            height,
            planes: [g, b, r],
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
}
