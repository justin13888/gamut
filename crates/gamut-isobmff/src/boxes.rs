//! A minimal append-only ISOBMFF box serializer with size back-patching.
//!
//! All boxes here are byte-aligned, so this works on a plain byte buffer rather than the bit-level
//! [`gamut_bitstream::BitWriter`]. `begin_box` returns the position of the box's size field; the
//! matching `end_box` back-patches it once the body is written. [`BoxBuilder::reserve_u32`] /
//! [`BoxBuilder::patch_u32`] support the `iloc` `extent_offset`, which can only be filled once the
//! `mdat` payload position is known.

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
