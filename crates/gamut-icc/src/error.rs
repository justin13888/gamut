//! The crate's error type.

/// An error from parsing or serializing an ICC profile.
///
/// Every failure the ICC reader and writer raise is a malformed input or a violated spec
/// invariant; the message names the specific structure at fault (e.g. `"icc: duplicate tag
/// signature"`). Exposing the crate's own type — rather than the shared [`gamut_core::Error`] —
/// keeps the failing carrier identifiable when embedded in a wider metadata pipeline.
///
/// Marked `#[non_exhaustive]` so a future failure category can be added without a breaking change.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IccError {
    /// The profile was malformed, truncated, or violated an ICC.1:2022 invariant. The string
    /// names the specific structure at fault.
    #[error("{0}")]
    Malformed(&'static str),
}

/// A [`Result`](core::result::Result) whose error is [`IccError`].
pub type Result<T> = core::result::Result<T, IccError>;
