//! The IFD reader.

/// Reader for a TIFF/IFD stream.
///
/// Will parse the [`crate::TiffHeader`], then follow the first-IFD offset and each entry's
/// value-or-offset word to materialise the [`crate::Ifd`] chain and any sub-IFDs. Because the
/// structure is offset-driven (a classic parser-exploit surface) the reader is built to be robust
/// against malformed entries, offset loops, and truncation. Implementation pending (see issue #34).
pub struct IfdReader;
