//! Pixel-sample helpers shared across the gamut codecs.

/// Saturates a computed integer sample to the unsigned 8-bit pixel range `0..=255`.
///
/// This is the single home for the `clip_pixel` / `clamp255` operation every codec performs when
/// writing a reconstructed sample (prediction + residual, loop-filter output, color conversion)
/// back to an 8-bit plane. It is a `const fn` using explicit branches rather than [`Ord::clamp`]
/// so it can be used in `const` contexts and stays friendly to later branchless/SIMD tuning.
#[must_use]
pub const fn clip_pixel8(x: i32) -> u8 {
    if x < 0 {
        0
    } else if x > 255 {
        255
    } else {
        x as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_pixel8_saturates() {
        assert_eq!(clip_pixel8(-1), 0);
        assert_eq!(clip_pixel8(-1000), 0);
        assert_eq!(clip_pixel8(0), 0);
        assert_eq!(clip_pixel8(128), 128);
        assert_eq!(clip_pixel8(255), 255);
        assert_eq!(clip_pixel8(1000), 255);
    }
}
