//! VP8 intra prediction modes (RFC 6386 §11-§12). Key-frame intra only.

/// Luma 16×16 prediction mode (RFC 6386 §11.2, §12.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LumaMode {
    /// DC (average of available top/left edges) prediction.
    Dc,
    /// Vertical prediction from the row above.
    Vertical,
    /// Horizontal prediction from the column to the left.
    Horizontal,
    /// TrueMotion prediction (top row + left column − top-left corner).
    TrueMotion,
    /// Per-4×4-subblock prediction; selects a [`SubBlockMode`] for each of the 16 subblocks.
    BPred,
}

/// Luma 4×4 subblock prediction mode, used when the macroblock mode is [`LumaMode::BPred`]
/// (RFC 6386 §11.2, §12.3). Ten directional / averaging modes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubBlockMode {
    /// DC (average) prediction.
    Dc,
    /// TrueMotion prediction.
    TrueMotion,
    /// Vertical prediction.
    Vertical,
    /// Horizontal prediction.
    Horizontal,
    /// Down-left diagonal prediction.
    LeftDown,
    /// Down-right diagonal prediction.
    RightDown,
    /// Vertical-right diagonal prediction.
    VerticalRight,
    /// Vertical-left diagonal prediction.
    VerticalLeft,
    /// Horizontal-down diagonal prediction.
    HorizontalDown,
    /// Horizontal-up diagonal prediction.
    HorizontalUp,
}

/// Chroma 8×8 prediction mode (RFC 6386 §12.2). The same four modes as the luma 16×16 set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromaMode {
    /// DC (average) prediction.
    Dc,
    /// Vertical prediction.
    Vertical,
    /// Horizontal prediction.
    Horizontal,
    /// TrueMotion prediction.
    TrueMotion,
}
