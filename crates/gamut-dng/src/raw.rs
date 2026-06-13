//! The raw sensor image an encoder consumes: the sample buffer plus the colour-filter-array
//! description and the black/white levels needed to interpret it.

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

/// A raw, single-plane **colour-filter-array (mosaic)** sensor image plus the metadata required to
/// store it as a DNG raw sub-IFD.
///
/// Samples are one unsigned integer per sensel, row-major, `width * height` long. The CFA pattern
/// (e.g. `[R, G, G, B]` over a `2 × 2` repeat) names the colour of each sensel; `cfa_plane_color`
/// lists the distinct plane colours (e.g. `[R, G, B]`). Black/white levels bound the linear range.
///
/// Built with [`RawImage::new_cfa`] (which validates the buffer and pattern sizes) and refined with
/// the `with_*` setters; full bit-depth packing, multi-value levels, and linearisation arrive in
/// later phases (see `STATUS.md`).
#[derive(Debug, Clone)]
pub struct RawImage {
    dims: Dimensions,
    bits_per_sample: u16,
    black_level: u32,
    white_level: u32,
    cfa_repeat: (u16, u16),
    cfa_pattern: Vec<u8>,
    cfa_plane_color: Vec<u8>,
    cfa_layout: CfaLayout,
    active_area: Option<[u32; 4]>,
    samples: Vec<u16>,
}

impl RawImage {
    /// Creates a CFA raw image from a mosaic `samples` buffer.
    ///
    /// `cfa_repeat` is `(rows, cols)` of the repeating pattern tile and `cfa_pattern` lists its
    /// colours in row-major order (length `rows * cols`). Defaults: black level `0`, white level
    /// `2^bits_per_sample - 1`, plane colours `[R, G, B]`, rectangular layout, full active area.
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
        if !(1..=16).contains(&bits_per_sample) {
            return Err(Error::InvalidInput("DNG: bits_per_sample must be 1..=16"));
        }
        let (rr, rc) = cfa_repeat;
        if rr == 0 || rc == 0 || cfa_pattern.len() != usize::from(rr) * usize::from(rc) {
            return Err(Error::InvalidInput(
                "DNG: CFA pattern length must equal cfa_repeat rows * cols",
            ));
        }
        let expected = dims
            .num_pixels()
            .ok_or(Error::InvalidInput("DNG: image dimensions overflow"))?;
        if samples.len() != expected {
            return Err(Error::InvalidInput(
                "DNG: sample count must equal width * height",
            ));
        }
        Ok(Self {
            dims,
            bits_per_sample,
            black_level: 0,
            white_level: white_level_default(bits_per_sample),
            cfa_repeat,
            cfa_pattern,
            cfa_plane_color: vec![cfa_color::RED, cfa_color::GREEN, cfa_color::BLUE],
            cfa_layout: CfaLayout::Rectangular,
            active_area: None,
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

    /// Sets the distinct CFA plane colours (default `[R, G, B]`). Returns `self` for chaining.
    #[must_use]
    pub fn with_cfa_plane_color(mut self, cfa_plane_color: Vec<u8>) -> Self {
        self.cfa_plane_color = cfa_plane_color;
        self
    }

    /// Sets the CFA layout (default [`CfaLayout::Rectangular`]). Returns `self` for chaining.
    #[must_use]
    pub fn with_cfa_layout(mut self, cfa_layout: CfaLayout) -> Self {
        self.cfa_layout = cfa_layout;
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

    /// The CFA repeat-pattern dimensions `(rows, cols)`.
    #[must_use]
    pub fn cfa_repeat(&self) -> (u16, u16) {
        self.cfa_repeat
    }

    /// The CFA pattern colours, row-major over the repeat tile.
    #[must_use]
    pub fn cfa_pattern(&self) -> &[u8] {
        &self.cfa_pattern
    }

    /// The distinct CFA plane colours.
    #[must_use]
    pub fn cfa_plane_color(&self) -> &[u8] {
        &self.cfa_plane_color
    }

    /// The CFA layout.
    #[must_use]
    pub fn cfa_layout(&self) -> CfaLayout {
        self.cfa_layout
    }

    /// The active-area rectangle `[top, left, bottom, right]`, if set.
    #[must_use]
    pub fn active_area(&self) -> Option<[u32; 4]> {
        self.active_area
    }

    /// The mosaic samples, row-major, `width * height` long.
    #[must_use]
    pub fn samples(&self) -> &[u16] {
        &self.samples
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
        assert_eq!(raw.black_level(), 0);
        assert_eq!(raw.cfa_plane_color(), &[0, 1, 2]);
        assert_eq!(raw.cfa_layout(), CfaLayout::Rectangular);
    }

    #[test]
    fn new_cfa_rejects_bad_sizes() {
        // Wrong sample count.
        assert!(RawImage::new_cfa(dims(4, 4), 16, (2, 2), vec![0, 1, 1, 2], vec![0; 15]).is_err());
        // Pattern length mismatch.
        assert!(RawImage::new_cfa(dims(4, 4), 16, (2, 2), vec![0, 1, 1], vec![0; 16]).is_err());
        // Out-of-range bit depth.
        assert!(RawImage::new_cfa(dims(4, 4), 17, (2, 2), vec![0, 1, 1, 2], vec![0; 16]).is_err());
    }

    #[test]
    fn setters_chain() {
        let raw = RawImage::new_cfa(dims(2, 2), 12, (2, 2), vec![0, 1, 1, 2], vec![0; 4])
            .unwrap()
            .with_black_level(64)
            .with_white_level(4095)
            .with_active_area([0, 0, 2, 2]);
        assert_eq!(raw.black_level(), 64);
        assert_eq!(raw.white_level(), 4095);
        assert_eq!(raw.active_area(), Some([0, 0, 2, 2]));
    }
}
