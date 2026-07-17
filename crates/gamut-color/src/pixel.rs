//! Pixel-sample helpers shared across the gamut codecs.

/// Saturates a computed integer sample to the unsigned 8-bit pixel range `0..=255`.
///
/// This is the single home for the `clip_pixel` / `clamp255` operation every codec performs when
/// writing a reconstructed sample (prediction + residual, loop-filter output, color conversion)
/// back to an 8-bit plane — one definition for later branchless/SIMD tuning. The `as u8` cast is
/// lossless because [`Ord::clamp`] has already constrained the value to `0..=255`.
#[must_use]
pub fn clip_pixel8(x: i32) -> u8 {
    x.clamp(0, 255) as u8
}

/// Saturates a computed integer sample to the unsigned `bit_depth`-bit pixel range
/// `0..=(1 << bit_depth) - 1` (AV1 `Clip1`, §3) — the high-bit-depth companion to [`clip_pixel8`].
///
/// `bit_depth` is 8, 10, 12, or 16 (the [`BitDepth`](crate::BitDepth) values); the result fits a
/// `u16` for all of them. At `bit_depth == 8` this equals `u16::from(clip_pixel8(x))`. AV1 itself
/// only reaches 12 — 16 is here for the wider still-image pipelines that share this vocabulary.
///
/// # Panics
///
/// Debug builds assert the `bit_depth` contract; release builds do not check it (this sits on
/// codec reconstruction hot paths), and depths outside 8/10/12/16 give meaningless results.
#[must_use]
pub fn clip_pixel(x: i32, bit_depth: u32) -> u16 {
    debug_assert!(
        matches!(bit_depth, 8 | 10 | 12 | 16),
        "clip_pixel bit_depth must be 8, 10, 12, or 16"
    );
    let max = (1i32 << bit_depth) - 1;
    x.clamp(0, max) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_pixel_saturates_per_bit_depth() {
        // 8-bit: clip_pixel8 saturates at 0/255, and clip_pixel(x, 8) agrees with it exactly;
        // 10/12-bit use the wider ceilings.
        for &(x, expect8) in &[
            (-1000, 0u16),
            (-1, 0),
            (0, 0),
            (128, 128),
            (200, 200),
            (255, 255),
            (1000, 255),
        ] {
            assert_eq!(u16::from(clip_pixel8(x)), expect8);
            assert_eq!(clip_pixel(x, 8), expect8);
        }
        assert_eq!(clip_pixel(1023, 10), 1023);
        assert_eq!(clip_pixel(2000, 10), 1023); // saturates at 2^10 - 1
        assert_eq!(clip_pixel(-1, 10), 0);
        assert_eq!(clip_pixel(4095, 12), 4095);
        assert_eq!(clip_pixel(9000, 12), 4095); // saturates at 2^12 - 1
        assert_eq!(clip_pixel(65535, 16), 65535);
        assert_eq!(clip_pixel(100_000, 16), 65535); // saturates at 2^16 - 1, the u16 ceiling
        assert_eq!(clip_pixel(-1, 16), 0);
    }
}
