//! VP8L bit I/O: an **LSB-first** bit stream (RFC 9649 §3.3).
//!
//! VP8L reads values least-significant-bit-first within each byte, which is the opposite order to
//! [`gamut_bitstream::BitWriter`](https://docs.rs/gamut-bitstream)'s MSB-first `f(n)` fields. The
//! reader (`ReadBits(n)`) and the matching writer land here at milestone M0; if a second consumer
//! (e.g. a future JPEG/JXL path) needs LSB-first bit I/O, this is a candidate to graduate into
//! `gamut-bitstream`. Tracked in `../STATUS.md` section F.
