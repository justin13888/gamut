//! VP8L LZ77 backward references (RFC 9649 §3.6.2).
//!
//! Beyond literal ARGB pixels, VP8L codes (length, distance) backward references into the pixels
//! already produced, with the 2-D distance mapped to a 1-D distance code via a fixed plane table.
//! The reference-finding (encoder) and reference-applying (decoder) logic lands here at milestone M1
//! (tracked in `../STATUS.md` section D).
