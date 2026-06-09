//! VP8 in-loop deblocking filter (RFC 6386 §15).
//!
//! After reconstruction, VP8 smooths macroblock and subblock edges with either a simple filter
//! (§15.2) or a normal filter with a high-edge-variance test (§15.3); per-macroblock control
//! parameters (level, interior/edge limits, segment adjustments) are derived per §15.4. Lands at
//! milestone M2 (tracked in `../STATUS.md` section M).
