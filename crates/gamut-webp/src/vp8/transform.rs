//! VP8 inverse (and forward) transforms (RFC 6386 §14.3-§14.4).
//!
//! VP8 reconstruction uses an integer 4×4 DCT for the luma/chroma residual subblocks and a 4×4
//! Walsh-Hadamard transform for the Y2 block of luma DC coefficients. **These are VP8-specific and
//! differ from AV1's WHT in `gamut-dsp`**, so they live in-crate (no second consumer). The forward
//! transforms used by the encoder land alongside them at milestone M2. Tracked in `../STATUS.md`
//! section L.
