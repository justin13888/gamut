//! The raw sensor image an encoder consumes: the sample buffer plus the photometry (CFA mosaic or
//! demosaiced linear) and the black/white levels needed to interpret it.

use gamut_core::{Dimensions, Error, Result};

use crate::values::CfaLayout;

/// CFA colour codes, as stored in the `CFAPattern` tag (DNG spec / TIFF-EP).
pub mod cfa_color {
    /// Red.
    pub const RED: u8 = 0;
    /// Green.
    pub const GREEN: u8 = 1;
    /// Blue.
    pub const BLUE: u8 = 2;
    /// Cyan.
    pub const CYAN: u8 = 3;
    /// Magenta.
    pub const MAGENTA: u8 = 4;
    /// Yellow.
    pub const YELLOW: u8 = 5;
    /// White.
    pub const WHITE: u8 = 6;
}

/// How a raw image's samples map to colour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawPhotometry {
    /// A colour-filter-array (mosaic) image: one sample per pixel, its colour given by the
    /// repeating pattern. Stored with `PhotometricInterpretation = CFA` and one sample per pixel.
    Cfa {
        /// The repeat-pattern dimensions `(rows, cols)` (e.g. `(2, 2)` for Bayer).
        repeat: (u16, u16),
        /// The pattern colours, row-major over the repeat tile (length `rows * cols`).
        pattern: Vec<u8>,
        /// The distinct CFA plane colours (e.g. `[R, G, B]`).
        plane_color: Vec<u8>,
        /// The physical CFA layout.
        layout: CfaLayout,
    },
    /// A demosaiced ("linear") image: `planes` interleaved samples per pixel. Stored with
    /// `PhotometricInterpretation = LinearRaw`.
    LinearRaw {
        /// Colour planes per pixel (e.g. 3 for RGB).
        planes: u16,
    },
}

/// A raw sensor image plus the metadata required to store it as a DNG raw sub-IFD.
///
/// Samples are unsigned integers, row-major, `width * height * samples_per_pixel` long — one per
/// pixel for a [`Cfa`](RawPhotometry::Cfa) mosaic, `planes` interleaved per pixel for
/// [`LinearRaw`](RawPhotometry::LinearRaw). Black/white levels bound the linear range.
///
/// Built with [`RawImage::new_cfa`] or [`RawImage::new_linear_raw`] (which validate the buffer and
/// pattern) and refined with the `with_*` setters.
#[derive(Debug, Clone)]
pub struct RawImage {
    dims: Dimensions,
    bits_per_sample: u16,
    samples_per_pixel: u16,
    black_level: u32,
    white_level: u32,
    active_area: Option<[u32; 4]>,
    photometry: RawPhotometry,
    samples: Vec<u16>,
}

impl RawImage {
    /// Creates a CFA (mosaic) raw image from a single-plane `samples` buffer.
    ///
    /// `cfa_repeat` is `(rows, cols)` of the repeating pattern tile and `cfa_pattern` lists its
    /// colours row-major (length `rows * cols`). Defaults: black `0`, white `2^bits - 1`, plane
    /// colours `[R, G, B]`, rectangular layout, full active area.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `bits_per_sample` is not in `1..=16`, the pattern length
    /// does not match `cfa_repeat`, or `samples.len()` is not `width * height`.
    pub fn new_cfa(
        dims: Dimensions,
        bits_per_sample: u16,
        cfa_repeat: (u16, u16),
        cfa_pattern: Vec<u8>,
        samples: Vec<u16>,
    ) -> Result<Self> {
        check_bits(bits_per_sample)?;
        let (rr, rc) = cfa_repeat;
        if rr == 0 || rc == 0 || cfa_pattern.len() != usize::from(rr) * usize::from(rc) {
            return Err(Error::InvalidInput(
                "DNG: CFA pattern length must equal cfa_repeat rows * cols",
            ));
        }
        check_sample_count(dims, 1, &samples)?;
        Ok(Self {
            dims,
            bits_per_sample,
            samples_per_pixel: 1,
            black_level: 0,
            white_level: white_level_default(bits_per_sample),
            active_area: None,
            photometry: RawPhotometry::Cfa {
                repeat: cfa_repeat,
                pattern: cfa_pattern,
                plane_color: vec![cfa_color::RED, cfa_color::GREEN, cfa_color::BLUE],
                layout: CfaLayout::Rectangular,
            },
            samples,
        })
    }

    /// Creates a demosaiced ("linear") raw image of `planes` interleaved samples per pixel.
    ///
    /// Defaults: black `0`, white `2^bits - 1`, full active area.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `bits_per_sample` is not in `1..=16`, `planes` is zero,
    /// or `samples.len()` is not `width * height * planes`.
    pub fn new_linear_raw(
        dims: Dimensions,
        bits_per_sample: u16,
        planes: u16,
        samples: Vec<u16>,
    ) -> Result<Self> {
        check_bits(bits_per_sample)?;
        if planes == 0 {
            return Err(Error::InvalidInput(
                "DNG: LinearRaw needs at least one plane",
            ));
        }
        check_sample_count(dims, planes, &samples)?;
        Ok(Self {
            dims,
            bits_per_sample,
            samples_per_pixel: planes,
            black_level: 0,
            white_level: white_level_default(bits_per_sample),
            active_area: None,
            photometry: RawPhotometry::LinearRaw { planes },
            samples,
        })
    }

    /// Sets the black level (the zero-light encoding value). Returns `self` for chaining.
    #[must_use]
    pub fn with_black_level(mut self, black_level: u32) -> Self {
        self.black_level = black_level;
        self
    }

    /// Sets the white level (the saturated encoding value). Returns `self` for chaining.
    #[must_use]
    pub fn with_white_level(mut self, white_level: u32) -> Self {
        self.white_level = white_level;
        self
    }

    /// Sets the active-area rectangle `[top, left, bottom, right]` (the region holding image data,
    /// excluding masked/border pixels). Returns `self` for chaining.
    #[must_use]
    pub fn with_active_area(mut self, active_area: [u32; 4]) -> Self {
        self.active_area = Some(active_area);
        self
    }

    /// Sets the distinct CFA plane colours (no effect on a linear image). Returns `self`.
    #[must_use]
    pub fn with_cfa_plane_color(mut self, colors: Vec<u8>) -> Self {
        if let RawPhotometry::Cfa { plane_color, .. } = &mut self.photometry {
            *plane_color = colors;
        }
        self
    }

    /// Sets the CFA layout (no effect on a linear image). Returns `self`.
    #[must_use]
    pub fn with_cfa_layout(mut self, cfa_layout: CfaLayout) -> Self {
        if let RawPhotometry::Cfa { layout, .. } = &mut self.photometry {
            *layout = cfa_layout;
        }
        self
    }

    /// The sensor sample dimensions.
    #[must_use]
    pub fn dimensions(&self) -> Dimensions {
        self.dims
    }

    /// Bits per stored sample.
    #[must_use]
    pub fn bits_per_sample(&self) -> u16 {
        self.bits_per_sample
    }

    /// Samples per pixel (1 for a CFA mosaic, the plane count for a linear image).
    #[must_use]
    pub fn samples_per_pixel(&self) -> u16 {
        self.samples_per_pixel
    }

    /// The black level (zero-light encoding value).
    #[must_use]
    pub fn black_level(&self) -> u32 {
        self.black_level
    }

    /// The white level (saturated encoding value).
    #[must_use]
    pub fn white_level(&self) -> u32 {
        self.white_level
    }

    /// The active-area rectangle `[top, left, bottom, right]`, if set.
    #[must_use]
    pub fn active_area(&self) -> Option<[u32; 4]> {
        self.active_area
    }

    /// The image's photometry (CFA mosaic or linear).
    #[must_use]
    pub fn photometry(&self) -> &RawPhotometry {
        &self.photometry
    }

    /// The samples, row-major, `width * height * samples_per_pixel` long.
    #[must_use]
    pub fn samples(&self) -> &[u16] {
        &self.samples
    }
}

/// Validates a bit depth is in the storable range.
fn check_bits(bits: u16) -> Result<()> {
    if (1..=16).contains(&bits) {
        Ok(())
    } else {
        Err(Error::InvalidInput("DNG: bits_per_sample must be 1..=16"))
    }
}

/// Validates `samples.len()` equals `width * height * spp`.
fn check_sample_count(dims: Dimensions, spp: u16, samples: &[u16]) -> Result<()> {
    let expected = dims
        .num_pixels()
        .and_then(|p| p.checked_mul(usize::from(spp)))
        .ok_or(Error::InvalidInput("DNG: image dimensions overflow"))?;
    if samples.len() == expected {
        Ok(())
    } else {
        Err(Error::InvalidInput(
            "DNG: sample count must equal width * height * samples_per_pixel",
        ))
    }
}

/// The default white level for `bits` bits per sample: `2^bits - 1`.
fn white_level_default(bits: u16) -> u32 {
    (1u32 << bits) - 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dims(w: u32, h: u32) -> Dimensions {
        Dimensions::new(w, h).unwrap()
    }

    #[test]
    fn new_cfa_validates_and_defaults() {
        let raw = RawImage::new_cfa(
            dims(4, 4),
            16,
            (2, 2),
            vec![
                cfa_color::RED,
                cfa_color::GREEN,
                cfa_color::GREEN,
                cfa_color::BLUE,
            ],
            vec![0u16; 16],
        )
        .expect("valid");
        assert_eq!(raw.white_level(), 65535);
        assert_eq!(raw.samples_per_pixel(), 1);
        assert!(matches!(raw.photometry(), RawPhotometry::Cfa { .. }));
    }

    #[test]
    fn new_linear_raw_validates() {
        let raw = RawImage::new_linear_raw(dims(2, 2), 16, 3, vec![0u16; 12]).expect("valid");
        assert_eq!(raw.samples_per_pixel(), 3);
        assert_eq!(raw.photometry(), &RawPhotometry::LinearRaw { planes: 3 });
        // Wrong sample count (needs w*h*planes = 12).
        assert!(RawImage::new_linear_raw(dims(2, 2), 16, 3, vec![0; 11]).is_err());
        assert!(RawImage::new_linear_raw(dims(2, 2), 16, 0, vec![]).is_err());
    }

    #[test]
    fn new_cfa_rejects_bad_sizes() {
        assert!(RawImage::new_cfa(dims(4, 4), 16, (2, 2), vec![0, 1, 1, 2], vec![0; 15]).is_err());
        assert!(RawImage::new_cfa(dims(4, 4), 16, (2, 2), vec![0, 1, 1], vec![0; 16]).is_err());
        assert!(RawImage::new_cfa(dims(4, 4), 17, (2, 2), vec![0, 1, 1, 2], vec![0; 16]).is_err());
    }

    #[test]
    fn setters_chain() {
        let raw = RawImage::new_cfa(dims(2, 2), 12, (2, 2), vec![0, 1, 1, 2], vec![0; 4])
            .unwrap()
            .with_black_level(64)
            .with_white_level(4095)
            .with_active_area([0, 0, 2, 2])
            .with_cfa_layout(CfaLayout::Rectangular);
        assert_eq!(raw.black_level(), 64);
        assert_eq!(raw.white_level(), 4095);
        assert_eq!(raw.active_area(), Some([0, 0, 2, 2]));
    }
}
