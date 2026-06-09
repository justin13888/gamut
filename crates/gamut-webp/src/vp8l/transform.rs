//! VP8L transforms: reversible pixel-decorrelation passes applied before entropy coding and undone
//! in reverse order on decode (RFC 9649 §3.5).

/// A VP8L transform type. Up to four transforms may be chained; on decode they are inverted in the
/// reverse of the order they were applied (RFC 9649 §3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Vp8lTransform {
    /// Predictor (spatial) transform: each pixel is predicted from its neighbors using one of 14
    /// modes selected per block.
    Predictor,
    /// Color transform: decorrelates the red and blue channels from green, per block.
    Color,
    /// Subtract-green transform: subtracts green from red and blue (a fixed special case of
    /// [`Vp8lTransform::Color`], cheap enough to warrant its own type).
    SubtractGreen,
    /// Color-indexing (palette) transform: replaces pixel values with indices into a color table.
    ColorIndexing,
}
