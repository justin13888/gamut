//! VP8L color cache (RFC 9649 §3.6.3).
//!
//! The color cache is a small hash table of recently emitted ARGB colors; a pixel can be coded as a
//! short index into it instead of a literal or a backward reference. The cache (a multiplicative
//! hash with a configurable bit width) lands here at milestone M1 (tracked in `../STATUS.md`
//! section D).
