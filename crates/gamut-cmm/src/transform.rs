//! The object-safe entry trait every runnable colour transform implements.

use crate::error::Result;

/// A runnable colour transform: interleaved `f64` pixels in, interleaved `f64` pixels out.
///
/// The single entry point every CMM product implements — a [`Pipeline`](crate::Pipeline) today;
/// linked profile transforms and chains in later phases. Object-safe by design (the
/// `gamut_heic::HevcDecoder` shape: one dispatchable method over borrowed data, plain data out),
/// so a transform can be boxed, held behind `&dyn Transform`, and later carried over the
/// C-portable seam.
///
/// # Buffer contract
///
/// `src` must hold `pixels × input_channels()` samples and `dst` exactly
/// `pixels × output_channels()` samples **for the same pixel count**; violations return
/// [`CmmError::BufferLength`](crate::CmmError::BufferLength). Samples are interleaved per pixel
/// (e.g. `RGBRGB…`), never planar.
///
/// # Sample domain
///
/// Device channels are **encoded** values in `[0.0, 1.0]`; PCS seams are **decoded
/// colorimetry** — PCSXYZ carries XYZ with D50 luminance `Y = 1.0`, PCSLAB carries `L*` in
/// `0..=100` and `a*`/`b*` in their natural signed range. See the crate-level docs; every
/// stage added by later phases keeps this convention.
pub trait Transform {
    /// Transforms `src` into `dst`, pixel by pixel.
    ///
    /// # Errors
    ///
    /// Returns [`CmmError::BufferLength`](crate::CmmError::BufferLength) if `src` is not a
    /// whole number of `input_channels()`-sample pixels, or `dst` does not hold exactly the
    /// matching number of `output_channels()`-sample pixels.
    fn transform(&self, src: &[f64], dst: &mut [f64]) -> Result<()>;

    /// The number of samples this transform consumes per pixel.
    #[must_use]
    fn input_channels(&self) -> u8;

    /// The number of samples this transform produces per pixel.
    #[must_use]
    fn output_channels(&self) -> u8;
}

#[cfg(test)]
mod tests {
    use super::Transform;
    use crate::{Pipeline, Stage};

    #[test]
    fn transform_is_object_safe() {
        let pipeline = Pipeline::new(3, 3, vec![Stage::Clamp { channels: 3 }]).unwrap();
        let dynamic: &dyn Transform = &pipeline;
        assert_eq!(dynamic.input_channels(), 3);
        assert_eq!(dynamic.output_channels(), 3);
        let mut dst = [0.0; 3];
        dynamic.transform(&[2.0, -1.0, 0.5], &mut dst).unwrap();
        assert_eq!(dst, [1.0, 0.0, 0.5]);
    }
}
