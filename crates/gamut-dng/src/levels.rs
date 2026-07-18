//! The DNG level model: the `BlackLevel` family, per-plane `WhiteLevel`, and the optional
//! `LinearizationTable` — the inputs to the spec's "Mapping Raw Values to Linear Reference
//! Values" pipeline (DNG 1.7.1 Chapter 5).
//!
//! DNG describes the sensor's encoding range with a *pattern* of black levels, not a scalar: a
//! `BlackLevelRepeatDim`-sized tile of per-cell, per-plane values (`BlackLevel`, spec p. 28)
//! anchored at the **active-area** top-left, refined by per-column (`BlackLevelDeltaH`) and
//! per-row (`BlackLevelDeltaV`) deltas, with a per-plane saturation value (`WhiteLevel`). An
//! optional `LinearizationTable` maps stored values through a lookup curve before any of that
//! applies. [`RawLevels`] models the whole family; the common uniform case is
//! [`RawLevels::uniform`].

use gamut_core::{Error, Result};

/// The DNG level family for one raw image: the `BlackLevel` pattern (+ optional per-column /
/// per-row deltas), the per-plane `WhiteLevel`, and the optional `LinearizationTable`.
///
/// Black levels may be fractional (the tag is RATIONAL-capable), so values are `f64`. The black
/// pattern is `rows * cols * samples_per_pixel` values in row-column-sample scan order, and — per
/// spec — repeats anchored at the **active area's** top-left corner, not the image origin. Delta
/// lengths are tied to the active-area geometry, which this model alone does not know; they are
/// validated where the geometry is available (encode and [`RawImage::to_linear`]).
///
/// [`RawImage::to_linear`]: crate::RawImage::to_linear
#[derive(Debug, Clone, PartialEq)]
pub struct RawLevels {
    black_repeat: (u16, u16),
    black: Vec<f64>,
    black_delta_h: Option<Vec<f64>>,
    black_delta_v: Option<Vec<f64>>,
    white: Vec<f64>,
    linearization_table: Option<Vec<u16>>,
}

impl RawLevels {
    /// Creates the common uniform model: one black and one white level shared by every pattern
    /// cell and sample plane (`BlackLevelRepeatDim = 1 × 1`).
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `samples_per_pixel` is zero, `black` is negative or not
    /// finite, or `white` is not finite and positive.
    pub fn uniform(samples_per_pixel: u16, black: f64, white: f64) -> Result<Self> {
        let spp = usize::from(samples_per_pixel);
        Self::new(
            samples_per_pixel,
            (1, 1),
            vec![black; spp],
            vec![white; spp],
        )
    }

    /// Creates a full pattern model.
    ///
    /// `black_repeat` is the `BlackLevelRepeatDim` `(rows, cols)`; `black` holds
    /// `rows * cols * samples_per_pixel` values in row-column-sample scan order (DNG 1.7.1 p. 28);
    /// `white` holds one saturation value per sample plane (p. 29).
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `samples_per_pixel` or a repeat dimension is zero, the
    /// vector lengths don't match, a black level is negative or not finite, or a white level is
    /// not finite and positive.
    pub fn new(
        samples_per_pixel: u16,
        black_repeat: (u16, u16),
        black: Vec<f64>,
        white: Vec<f64>,
    ) -> Result<Self> {
        let (rows, cols) = black_repeat;
        if samples_per_pixel == 0 {
            return Err(Error::InvalidInput(
                "DNG: levels need at least one sample plane",
            ));
        }
        if rows == 0 || cols == 0 {
            return Err(Error::InvalidInput(
                "DNG: BlackLevelRepeatDim dimensions must be non-zero",
            ));
        }
        let cells = usize::from(rows) * usize::from(cols) * usize::from(samples_per_pixel);
        if black.len() != cells {
            return Err(Error::InvalidInput(
                "DNG: BlackLevel length must be repeat rows * cols * samples per pixel",
            ));
        }
        if white.len() != usize::from(samples_per_pixel) {
            return Err(Error::InvalidInput(
                "DNG: WhiteLevel needs one value per sample plane",
            ));
        }
        if black.iter().any(|v| !v.is_finite() || *v < 0.0) {
            return Err(Error::InvalidInput(
                "DNG: black levels must be finite and non-negative",
            ));
        }
        if white.iter().any(|v| !v.is_finite() || *v <= 0.0) {
            return Err(Error::InvalidInput(
                "DNG: white levels must be finite and positive",
            ));
        }
        Ok(Self {
            black_repeat,
            black,
            black_delta_h: None,
            black_delta_v: None,
            white,
            linearization_table: None,
        })
    }

    /// Sets the per-column black deltas (`BlackLevelDeltaH`) — one value per **active-area**
    /// column. The length is validated against the active area at encode / `to_linear` time.
    /// Returns `self` for chaining.
    #[must_use]
    pub fn with_black_delta_h(mut self, deltas: Vec<f64>) -> Self {
        self.black_delta_h = Some(deltas);
        self
    }

    /// Sets the per-row black deltas (`BlackLevelDeltaV`) — one value per **active-area** row.
    /// The length is validated against the active area at encode / `to_linear` time. Returns
    /// `self` for chaining.
    #[must_use]
    pub fn with_black_delta_v(mut self, deltas: Vec<f64>) -> Self {
        self.black_delta_v = Some(deltas);
        self
    }

    /// Sets the `LinearizationTable`: stored values index into it before black subtraction, and
    /// inputs at or beyond its length map to the last entry (DNG 1.7.1 pp. 27, 99). One table
    /// serves all sample planes. Returns `self` for chaining.
    #[must_use]
    pub fn with_linearization_table(mut self, table: Vec<u16>) -> Self {
        self.linearization_table = Some(table);
        self
    }

    /// The `BlackLevelRepeatDim` `(rows, cols)`.
    #[must_use]
    pub fn black_repeat(&self) -> (u16, u16) {
        self.black_repeat
    }

    /// The black-level pattern, `rows * cols * samples_per_pixel` values in row-column-sample
    /// scan order.
    #[must_use]
    pub fn black(&self) -> &[f64] {
        &self.black
    }

    /// The pattern black level for the pixel at (`row`, `col`) of `plane`, where `row`/`col` are
    /// **active-area-relative** coordinates (the pattern anchors at the active area's top-left,
    /// spec p. 28). Deltas are not included.
    ///
    /// Returns `None` if `plane >= samples_per_pixel()` (rows and columns wrap modulo the repeat
    /// pattern, so they cannot be out of range).
    #[must_use]
    pub fn black_at(&self, row: usize, col: usize, plane: usize) -> Option<f64> {
        let (rows, cols) = (
            usize::from(self.black_repeat.0),
            usize::from(self.black_repeat.1),
        );
        let spp = self.white.len();
        if plane >= spp {
            return None;
        }
        Some(self.black[((row % rows) * cols + (col % cols)) * spp + plane])
    }

    /// The per-column black deltas (`BlackLevelDeltaH`), if set.
    #[must_use]
    pub fn black_delta_h(&self) -> Option<&[f64]> {
        self.black_delta_h.as_deref()
    }

    /// The per-row black deltas (`BlackLevelDeltaV`), if set.
    #[must_use]
    pub fn black_delta_v(&self) -> Option<&[f64]> {
        self.black_delta_v.as_deref()
    }

    /// The per-plane white (saturation) levels.
    #[must_use]
    pub fn white(&self) -> &[f64] {
        &self.white
    }

    /// The `LinearizationTable`, if set.
    #[must_use]
    pub fn linearization_table(&self) -> Option<&[u16]> {
        self.linearization_table.as_deref()
    }

    /// The number of sample planes this model describes.
    #[must_use]
    pub fn samples_per_pixel(&self) -> u16 {
        self.white.len() as u16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_builds_one_cell_per_plane() {
        let levels = RawLevels::uniform(3, 64.0, 4095.0).expect("valid");
        assert_eq!(levels.black_repeat(), (1, 1));
        assert_eq!(levels.black(), &[64.0, 64.0, 64.0]);
        assert_eq!(levels.white(), &[4095.0, 4095.0, 4095.0]);
        assert_eq!(levels.samples_per_pixel(), 3);
        assert_eq!(levels.black_delta_h(), None);
        assert_eq!(levels.black_delta_v(), None);
        assert_eq!(levels.linearization_table(), None);
    }

    #[test]
    fn new_validates_lengths_and_ranges() {
        // Wrong black length (needs 2*2*1 = 4).
        assert!(RawLevels::new(1, (2, 2), vec![0.0; 3], vec![255.0]).is_err());
        // Wrong white length (needs one per plane).
        assert!(RawLevels::new(2, (1, 1), vec![0.0; 2], vec![255.0]).is_err());
        // Zero repeat dims / zero planes.
        assert!(RawLevels::new(1, (0, 2), vec![], vec![255.0]).is_err());
        assert!(RawLevels::new(1, (2, 0), vec![], vec![255.0]).is_err());
        assert!(RawLevels::new(0, (1, 1), vec![], vec![]).is_err());
        // Negative / non-finite black, non-positive / non-finite white.
        assert!(RawLevels::new(1, (1, 1), vec![-1.0], vec![255.0]).is_err());
        assert!(RawLevels::new(1, (1, 1), vec![f64::NAN], vec![255.0]).is_err());
        assert!(RawLevels::new(1, (1, 1), vec![0.0], vec![0.0]).is_err());
        assert!(RawLevels::new(1, (1, 1), vec![0.0], vec![f64::INFINITY]).is_err());
        // The valid boundary: black 0, white 1.
        assert!(RawLevels::new(1, (1, 1), vec![0.0], vec![1.0]).is_ok());
    }

    #[test]
    fn black_at_indexes_the_pattern_by_phase() {
        // 2x2 pattern, 1 plane, distinct values per cell: value = row-major cell index.
        let levels = RawLevels::new(1, (2, 2), vec![10.0, 20.0, 30.0, 40.0], vec![255.0]).unwrap();
        // Phase wraps modulo the repeat dims — (2, 3) has phase (0, 1).
        assert_eq!(levels.black_at(0, 0, 0), Some(10.0));
        assert_eq!(levels.black_at(0, 1, 0), Some(20.0));
        assert_eq!(levels.black_at(1, 0, 0), Some(30.0));
        assert_eq!(levels.black_at(1, 1, 0), Some(40.0));
        assert_eq!(levels.black_at(2, 3, 0), Some(20.0));
        assert_eq!(levels.black_at(3, 2, 0), Some(30.0));
        // A plane past `samples_per_pixel` is a miss, not a panic.
        assert_eq!(levels.black_at(0, 0, 1), None);
    }

    #[test]
    fn black_at_indexes_planes_within_a_cell() {
        // 1x2 pattern, 2 planes: cells are [c0p0, c0p1, c1p0, c1p1].
        let levels =
            RawLevels::new(2, (1, 2), vec![1.0, 2.0, 3.0, 4.0], vec![255.0, 255.0]).unwrap();
        assert_eq!(levels.black_at(0, 0, 0), Some(1.0));
        assert_eq!(levels.black_at(0, 0, 1), Some(2.0));
        assert_eq!(levels.black_at(0, 1, 0), Some(3.0));
        assert_eq!(levels.black_at(0, 1, 1), Some(4.0));
        // Rows all share the single pattern row.
        assert_eq!(levels.black_at(5, 0, 1), Some(2.0));
    }

    #[test]
    fn builders_attach_deltas_and_table() {
        let levels = RawLevels::uniform(1, 0.0, 255.0)
            .unwrap()
            .with_black_delta_h(vec![0.5, -0.5])
            .with_black_delta_v(vec![1.0])
            .with_linearization_table(vec![0, 1, 4, 9]);
        assert_eq!(levels.black_delta_h(), Some(&[0.5, -0.5][..]));
        assert_eq!(levels.black_delta_v(), Some(&[1.0][..]));
        assert_eq!(levels.linearization_table(), Some(&[0, 1, 4, 9][..]));
    }
}
