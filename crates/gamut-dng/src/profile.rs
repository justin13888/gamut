//! The camera colour profile an encoder writes into the DNG: the colour-calibration matrix, the
//! calibration illuminant, and the as-shot white balance.
//!
//! This is the minimal profile a colour DNG needs. The second/third illuminant, the camera- and
//! forward-calibration matrices, and the profile look/tone tables are layered on in a later phase
//! (see `STATUS.md`).

use gamut_core::{Error, Result};

use crate::values::CalibrationIlluminant;

/// The denominator used when storing a coordinate as a TIFF `RATIONAL`/`SRATIONAL`.
///
/// Six significant digits comfortably exceed the precision DNG colour math needs, and the matrix
/// entries (`|x| < 4`) and white-balance coordinates (`0..1`) stay well inside `i32`/`u32` range.
const RATIONAL_DEN: i32 = 1_000_000;

/// Converts a finite `f64` to a signed `RATIONAL` `(numerator, denominator)` pair.
#[must_use]
pub(crate) fn srational(x: f64) -> (i32, i32) {
    ((x * f64::from(RATIONAL_DEN)).round() as i32, RATIONAL_DEN)
}

/// Converts a non-negative `f64` to an unsigned `RATIONAL` `(numerator, denominator)` pair
/// (negatives clamp to zero).
#[must_use]
pub(crate) fn urational(x: f64) -> (u32, u32) {
    let den = RATIONAL_DEN as u32;
    ((x.max(0.0) * f64::from(den)).round() as u32, den)
}

/// A camera colour profile: how the sensor's native colours relate to CIE XYZ, and the white
/// balance the shot was taken under.
///
/// `color_matrix1` is the row-major `3 × 3` matrix mapping CIE XYZ (under `calibration_illuminant1`)
/// to the camera's native colour space, stored in the `ColorMatrix1` tag. `as_shot_neutral` is the
/// camera-native value of a neutral (grey) subject — the as-shot white balance — stored in
/// `AsShotNeutral`.
#[derive(Debug, Clone)]
pub struct CameraProfile {
    unique_camera_model: String,
    color_matrix1: [f64; 9],
    calibration_illuminant1: CalibrationIlluminant,
    as_shot_neutral: [f64; 3],
}

impl CameraProfile {
    /// Creates a profile for a 3-colour (RGB) camera.
    ///
    /// `color_matrix1` is the row-major `3 × 3` XYZ → camera-native matrix; `as_shot_neutral` is the
    /// 3-component as-shot neutral. `unique_camera_model` must be a non-empty, non-localized model
    /// name (the `UniqueCameraModel` tag, which raw processors key their calibration on).
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `unique_camera_model` is empty or any `as_shot_neutral`
    /// component is not strictly positive (a neutral coordinate must be a usable divisor).
    pub fn new(
        unique_camera_model: impl Into<String>,
        color_matrix1: [f64; 9],
        calibration_illuminant1: CalibrationIlluminant,
        as_shot_neutral: [f64; 3],
    ) -> Result<Self> {
        let unique_camera_model = unique_camera_model.into();
        if unique_camera_model.is_empty() {
            return Err(Error::InvalidInput(
                "DNG: UniqueCameraModel must not be empty",
            ));
        }
        if !as_shot_neutral.iter().all(|&n| n.is_finite() && n > 0.0) {
            return Err(Error::InvalidInput(
                "DNG: AsShotNeutral components must be positive",
            ));
        }
        Ok(Self {
            unique_camera_model,
            color_matrix1,
            calibration_illuminant1,
            as_shot_neutral,
        })
    }

    /// The non-localized unique camera model name.
    #[must_use]
    pub fn unique_camera_model(&self) -> &str {
        &self.unique_camera_model
    }

    /// The row-major `3 × 3` XYZ → camera-native colour matrix (`ColorMatrix1`).
    #[must_use]
    pub fn color_matrix1(&self) -> &[f64; 9] {
        &self.color_matrix1
    }

    /// The calibration illuminant for `color_matrix1`.
    #[must_use]
    pub fn calibration_illuminant1(&self) -> CalibrationIlluminant {
        self.calibration_illuminant1
    }

    /// The as-shot neutral (white balance) in camera-native coordinates.
    #[must_use]
    pub fn as_shot_neutral(&self) -> &[f64; 3] {
        &self.as_shot_neutral
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rational_helpers_round_trip_to_double() {
        let (n, d) = srational(-0.5);
        assert!((f64::from(n) / f64::from(d) + 0.5).abs() < 1e-9);
        let (n, d) = urational(0.8);
        assert!((f64::from(n) / f64::from(d) - 0.8).abs() < 1e-9);
        // Negatives clamp to zero for the unsigned form.
        assert_eq!(urational(-1.0), (0, RATIONAL_DEN as u32));
    }

    #[test]
    fn new_validates() {
        let m = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        assert!(CameraProfile::new("Cam", m, CalibrationIlluminant::D65, [0.5, 1.0, 0.6]).is_ok());
        assert!(CameraProfile::new("", m, CalibrationIlluminant::D65, [0.5, 1.0, 0.6]).is_err());
        assert!(CameraProfile::new("Cam", m, CalibrationIlluminant::D65, [0.0, 1.0, 0.6]).is_err());
    }
}
