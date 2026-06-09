//! VP8 boolean entropy coder (RFC 6386 §7).
//!
//! VP8 codes every header field and coefficient token with a binary arithmetic coder driven by
//! 8-bit probabilities — distinct from AV1's multi-symbol range coder in `gamut-bitstream`. The
//! encoder (`BoolEncoder`) and decoder (`BoolDecoder`) land here at milestone M2. As in
//! `gamut-bitstream/src/symbol.rs`, the decoder is both production code (gamut ships a decoder) and
//! the encoder's hermetic round-trip oracle. Tracked in `../STATUS.md` section G.
