//! VP8 dequantization (RFC 6386 §9.6, §14.1).
//!
//! The frame header carries a base quantizer index and per-plane deltas
//! ([`QuantIndices`](super::header::QuantIndices)); this module maps those indices through the fixed
//! DC/AC lookup tables to the per-plane dequant factors (Y1, Y2, UV) used during reconstruction, and
//! the matching forward quantization for the encoder. Lands at milestone M2 (tracked in
//! `../STATUS.md` section L).
