//! VP8L canonical prefix (Huffman) codes (RFC 9649 §3.7).
//!
//! VP8L entropy-codes symbols with canonical prefix codes built from per-symbol code lengths. A
//! prefix-code group bundles five codes (green+length, red, blue, alpha, distance), and meta prefix
//! codes select a group per block via an entropy image (§3.7.1-§3.7.3).
//!
//! The code builder, the decode table (which doubles as the encoder's round-trip oracle), and the
//! simple/normal code-length coding land here at milestones M0-M1 (tracked in `../STATUS.md`
//! section C). Canonical Huffman is generic enough to graduate into `gamut-bitstream` should a
//! second consumer appear.
