//! The IFD writer.

/// Writer for a TIFF/IFD stream.
///
/// Will serialise an [`crate::Ifd`] chain back to bytes: the 8-byte header, each directory's
/// ascending-tag-sorted entries with values packed inline (≤ 4 bytes) or appended out-of-line, and
/// the next-IFD links. The hard part — the crate's **keystone** — is the **two-pass offset
/// layout**: out-of-line values and following IFDs need absolute offsets that are only known once
/// sizes are fixed, so the writer plans the layout then back-patches the offset words. A round-trip
/// (read → write → read) must reproduce the directory exactly. Implementation pending (see
/// issue #34).
pub struct IfdWriter;
