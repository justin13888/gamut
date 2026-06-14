//! Low-level ISOBMFF box serialization ([`BoxBuilder`]) and parsing ([`BoxReader`]).
//!
//! All boxes here are byte-aligned big-endian, so these work on a plain byte buffer rather than a
//! bit-level writer. [`BoxBuilder::begin_box`] returns the position of the box's size field; the
//! matching [`BoxBuilder::end_box`] back-patches it once the body is written.
//! [`BoxBuilder::reserve_u32`] / [`BoxBuilder::patch_u32`] support the `iloc` `extent_offset`, which
//! can only be filled once the `mdat` payload position is known. [`BoxReader`] is the read dual: a
//! bounds-checked cursor whose [`BoxReader::next_box`] walks a box list, never trusting a length
//! from the stream without checking it against the remaining bytes.

use gamut_core::{Error, Result};

/// Append-only big-endian box writer.
pub(crate) struct BoxBuilder {
    buf: Vec<u8>,
}

impl BoxBuilder {
    /// Creates an empty builder.
    pub(crate) fn new() -> Self {
        Self { buf: Vec::new() }
    }

    /// Opens a box of type `box_type`, writing a placeholder 32-bit size; returns the size field's
    /// position to pass to [`BoxBuilder::end_box`].
    pub(crate) fn begin_box(&mut self, box_type: &[u8; 4]) -> usize {
        let start = self.buf.len();
        self.buf.extend_from_slice(&[0, 0, 0, 0]);
        self.buf.extend_from_slice(box_type);
        start
    }

    /// Closes the box opened at `start`, back-patching its 32-bit size.
    pub(crate) fn end_box(&mut self, start: usize) {
        let size = (self.buf.len() - start) as u32;
        self.buf[start..start + 4].copy_from_slice(&size.to_be_bytes());
    }

    /// Writes a `FullBox` header (1-byte version, 3-byte flags).
    pub(crate) fn full_box(&mut self, version: u8, flags: u32) {
        self.buf.push(version);
        self.buf.extend_from_slice(&flags.to_be_bytes()[1..]);
    }

    pub(crate) fn u8(&mut self, value: u8) {
        self.buf.push(value);
    }

    pub(crate) fn u16(&mut self, value: u16) {
        self.buf.extend_from_slice(&value.to_be_bytes());
    }

    pub(crate) fn u32(&mut self, value: u32) {
        self.buf.extend_from_slice(&value.to_be_bytes());
    }

    pub(crate) fn bytes(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// Writes a 4-byte placeholder and returns its position for a later [`BoxBuilder::patch_u32`].
    pub(crate) fn reserve_u32(&mut self) -> usize {
        let pos = self.buf.len();
        self.buf.extend_from_slice(&[0, 0, 0, 0]);
        pos
    }

    /// Overwrites the 4 bytes at `pos` with `value` (big-endian).
    pub(crate) fn patch_u32(&mut self, pos: usize, value: u32) {
        self.buf[pos..pos + 4].copy_from_slice(&value.to_be_bytes());
    }

    /// Current length of the buffer (also the absolute file offset of the next byte written).
    pub(crate) fn len(&self) -> usize {
        self.buf.len()
    }

    /// Consumes the builder, returning the serialized bytes.
    pub(crate) fn into_vec(self) -> Vec<u8> {
        self.buf
    }
}

/// A bounds-checked big-endian cursor over a byte slice, used to parse a box (or a list of boxes).
///
/// Every read is checked against the remaining bytes, so a truncated or malformed stream yields a
/// typed [`Error`] rather than a panic.
pub(crate) struct BoxReader<'a> {
    data: &'a [u8],
    pos: usize,
}

/// One box returned by [`BoxReader::next_box`]: its four-character type and a borrow of its body.
pub(crate) struct RawBox<'a> {
    pub(crate) ty: [u8; 4],
    pub(crate) body: &'a [u8],
}

impl<'a> BoxReader<'a> {
    /// Creates a cursor at the start of `data`.
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Bytes not yet consumed.
    pub(crate) fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    /// Consumes `n` bytes, returning them, or [`Error::InvalidInput`] if fewer remain.
    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or(Error::InvalidInput("ISOBMFF: length overflow"))?;
        let slice = self
            .data
            .get(self.pos..end)
            .ok_or(Error::InvalidInput("ISOBMFF: unexpected end of box"))?;
        self.pos = end;
        Ok(slice)
    }

    /// Reads one byte.
    pub(crate) fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    /// Reads a big-endian `u16`.
    pub(crate) fn u16(&mut self) -> Result<u16> {
        let b = self.take(2)?;
        Ok(u16::from_be_bytes([b[0], b[1]]))
    }

    /// Reads a big-endian `u32`.
    pub(crate) fn u32(&mut self) -> Result<u32> {
        let b = self.take(4)?;
        Ok(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// Reads a four-character code.
    pub(crate) fn fourcc(&mut self) -> Result<[u8; 4]> {
        let b = self.take(4)?;
        Ok([b[0], b[1], b[2], b[3]])
    }

    /// Reads the next box header and body, advancing past it. Returns `Ok(None)` at a clean end of
    /// the slice.
    ///
    /// Only 32-bit box sizes are accepted: a `size == 1` (64-bit `largesize`) or `size == 0` (box
    /// extends to end-of-file) is rejected as [`Error::Unsupported`] — this crate never writes them,
    /// and accepting an unwritten path would leave it untested.
    pub(crate) fn next_box(&mut self) -> Result<Option<RawBox<'a>>> {
        if self.remaining() == 0 {
            return Ok(None);
        }
        let size = self.u32()? as usize;
        let ty = self.fourcc()?;
        match size {
            1 => return Err(Error::Unsupported("ISOBMFF: 64-bit box size (largesize)")),
            0 => return Err(Error::Unsupported("ISOBMFF: open-ended box (size 0)")),
            s if s < 8 => return Err(Error::InvalidInput("ISOBMFF: box size smaller than header")),
            _ => {}
        }
        let body = self.take(size - 8)?;
        Ok(Some(RawBox { ty, body }))
    }
}
