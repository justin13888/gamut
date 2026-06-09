//! VP8 DCT/WHT coefficient tokens (RFC 6386 §13).
//!
//! Each block's coefficients are coded as a tree of tokens over the boolean entropy coder. Tokens
//! `Zero..=Four` are literal magnitudes; the `Category1..=Category6` tokens introduce ranges of
//! larger magnitudes via a fixed number of extra probability-coded bits; `EndOfBlock` terminates the
//! block. The context-dependent probability tables live alongside this type at milestone M2.

/// A VP8 coefficient token (RFC 6386 §13.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    /// Literal coefficient magnitude 0.
    Zero,
    /// Literal coefficient magnitude 1.
    One,
    /// Literal coefficient magnitude 2.
    Two,
    /// Literal coefficient magnitude 3.
    Three,
    /// Literal coefficient magnitude 4.
    Four,
    /// Category 1: magnitude 5-6 (1 extra bit).
    Category1,
    /// Category 2: magnitude 7-10 (2 extra bits).
    Category2,
    /// Category 3: magnitude 11-18 (3 extra bits).
    Category3,
    /// Category 4: magnitude 19-34 (4 extra bits).
    Category4,
    /// Category 5: magnitude 35-66 (5 extra bits).
    Category5,
    /// Category 6: magnitude 67-2048 (11 extra bits).
    Category6,
    /// End of block: all remaining coefficients in scan order are zero.
    EndOfBlock,
}
