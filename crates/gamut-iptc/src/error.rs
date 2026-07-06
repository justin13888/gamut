//! The crate's error type.

/// An error from parsing or serializing IPTC photo metadata.
///
/// The two failure modes mirror the crate's error contract: [`Malformed`](IptcError::Malformed)
/// for a value that cannot be represented on the wire (strict write) or an IIM/IRB structure that
/// is corrupt (honest read), and [`Unsupported`](IptcError::Unsupported) for a `1:90`
/// coded-character-set designation gamut declines to guess at. Exposing the crate's own type —
/// rather than the shared [`gamut_core::Error`] — keeps the failing carrier identifiable when
/// embedded in a wider metadata pipeline.
///
/// Marked `#[non_exhaustive]` so a future failure category can be added without a breaking change.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum IptcError {
    /// A value could not be encoded on the wire, or an IIM/IRB structure was malformed. The string
    /// names the specific structure or constraint at fault.
    #[error("{0}")]
    Malformed(&'static str),
    /// A feature the crate declines to handle — chiefly a `1:90` coded-character-set designation
    /// beyond Latin-1 and UTF-8. The string names the unsupported feature.
    #[error("unsupported: {0}")]
    Unsupported(&'static str),
}

/// A [`Result`](core::result::Result) whose error is [`IptcError`].
pub type Result<T> = core::result::Result<T, IptcError>;
