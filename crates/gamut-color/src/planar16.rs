//! The high-bit-depth planar buffer: [`Planar16`], the 10/12-bit sibling of
//! [`Planar8`](crate::Planar8).
//!
//! Same geometry model, same plane order, wider samples. It exists rather than a generic
//! `Planar<T>` because the two are used at different boundaries — 8-bit encoding is the common
//! path and pays no `u16` widening for it — and because a high-bit-depth buffer carries one thing
//! an 8-bit buffer cannot need: the [`BitDepth`] its samples are coded at. `u16` is storage, `10`
//! or `12` is meaning, and every sample is validated against it at construction.

use gamut_core::{Dimensions, Error, Result};

use crate::format::{BitDepth, ChromaSubsampling};

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

    #[test]
    #[should_panic(expected = "plane index 3 out of range")]
    fn plane_dimensions_rejects_an_out_of_range_index() {
        let p = planes(BitDepth::Ten, vec![0; 6]).unwrap();
        let _ = p.plane_dimensions(3);
    }
}
