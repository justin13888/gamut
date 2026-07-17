//! Entropy-scan decoding (T.81 Annex F §F.2): the byte-destuffing bit reader (§F.2.2.5), the
//! canonical `DECODE`/`RECEIVE`/`EXTEND` procedures (Figures F.16, F.12), and the MCU walk that
//! reconstructs each component's sample plane (§A.2). Restart processing follows §E.2.5.
//!
//! Decoding is deliberately generous: every read is bounds-checked and every table lookup is
//! fallible, so malformed input yields a typed [`Error`] and never a panic. The only hard limits are
//! the spec's own — a Huffman code longer than 16 bits, a DC magnitude category above 11 (8-bit), or
//! an AC coefficient index past 63 — each of which is a validation error.

use gamut_core::{Error, Result};
use gamut_dsp::jpeg::idct8x8;

use crate::huffman::DecTable;
use crate::syntax::{Frame, ScanHeader, Tables};
use crate::zigzag::ZIGZAG;

/// One decoded component's reconstructed sample plane, stored at block-padded resolution: `stride`
/// is `blocks_per_line · 8` and rows grow in units of a block as the scan proceeds. The valid image
/// region (`comp_w × comp_h`) is cropped from this during upsampling.
pub struct Plane {
    /// Reconstructed clamped samples, row-major, `stride` wide.
    pub data: Vec<u8>,
    /// Row length in samples (`blocks_per_line · 8`).
    pub stride: usize,
    /// Horizontal sampling factor of the component (for later upsampling).
    pub h: u8,
    /// Vertical sampling factor of the component (for later upsampling).
    pub v: u8,
}

impl Plane {
    /// Grows the backing store so at least `rows` pixel rows exist (zero-filled). Written as a
    /// resize to `max(len, want)` — never a shrink — so there is no "already big enough" comparison
    /// whose boundary would be unobservable (a same-length resize is a no-op).
    fn ensure_rows(&mut self, rows: usize) {
        let want = rows * self.stride;
        self.data.resize(self.data.len().max(want), 0);
    }
}

/// The result of decoding one scan: each coded component's plane (paired with its frame index) and
/// the position of the marker that terminated the entropy data, where the segment parser resumes.
pub struct ScanResult {
    /// `(frame_component_index, plane)` for every component coded by this scan.
    pub planes: Vec<(usize, Plane)>,
    /// Byte offset of the terminating marker's `0xFF` prefix, where the segment parser resumes.
    pub marker_offset: usize,
}

/// The outcome of fetching one destuffed entropy byte.
enum Fetch {
    /// A data byte (any stuffed `0x00` after `0xFF` already consumed).
    Data(u8),
    /// A marker was reached; the reader's `marker`/`marker_off` are set and `pos` is frozen.
    Marker,
    /// The input ended without a terminating marker (truncated stream).
    Eof,
}

/// A most-significant-bit-first reader over the entropy-coded segment, applying §F.2.2.5 byte
/// destuffing (`0xFF 0x00` → literal `0xFF`) and stopping cleanly at any real marker (fill `0xFF`
/// bytes before a marker per §B.1.1.2 are skipped). Once a marker is reached, further bit requests
/// are satisfied with `1`-padding so the final valid symbols still decode; the byte cursor stays
/// parked on the marker.
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit_buf: u32,
    bit_count: u32,
    marker: Option<u8>,
    marker_off: usize,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8], start: usize) -> Self {
        Self {
            data,
            pos: start,
            bit_buf: 0,
            bit_count: 0,
            marker: None,
            marker_off: start,
        }
    }

    /// Fetches the next destuffed data byte, or reports a marker / EOF (§F.2.2.5).
    fn fetch(&mut self) -> Fetch {
        if self.marker.is_some() {
            return Fetch::Marker;
        }
        loop {
            let b = match self.data.get(self.pos) {
                Some(&b) => b,
                None => return Fetch::Eof,
            };
            if b != 0xFF {
                self.pos += 1;
                return Fetch::Data(b);
            }
            match self.data.get(self.pos + 1) {
                None => return Fetch::Eof,
                Some(0x00) => {
                    self.pos += 2;
                    return Fetch::Data(0xFF);
                }
                Some(0xFF) => {
                    // Fill byte before a marker (§B.1.1.2): skip it and re-examine.
                    self.pos += 1;
                }
                Some(&m) => {
                    self.marker = Some(m);
                    self.marker_off = self.pos;
                    return Fetch::Marker;
                }
            }
        }
    }

    /// Ensures at least `n` (`≤ 16`) bits are buffered, byte at a time, padding with `1`s past a
    /// marker. Errors only on a truncated stream (EOF with no marker).
    ///
    /// Each byte is composed with `+` rather than `|`: the shift just vacated 8 low zero bits and
    /// the addend is `< 256`, so the addition fills exactly those vacant bits (the same convention
    /// as `bitwriter`/`pack_nibbles`, where `|` and `^` would be equivalent on disjoint operands).
    fn ensure(&mut self, n: u32) -> Result<()> {
        while self.bit_count < n {
            match self.fetch() {
                Fetch::Data(b) => {
                    self.bit_buf = (self.bit_buf << 8) + u32::from(b);
                    self.bit_count += 8;
                }
                Fetch::Marker => {
                    self.bit_buf = (self.bit_buf << 8) + 0xFF;
                    self.bit_count += 8;
                }
                Fetch::Eof => {
                    return Err(Error::InvalidInput("JPEG: truncated entropy-coded data"));
                }
            }
        }
        Ok(())
    }

    /// Reads `n` (`0..=16`) bits MSB-first (`RECEIVE`, Figure F.12).
    fn read_bits(&mut self, n: u32) -> Result<u32> {
        if n == 0 {
            return Ok(0);
        }
        self.ensure(n)?;
        self.bit_count -= n;
        Ok((self.bit_buf >> self.bit_count) & ((1u32 << n) - 1))
    }

    /// Reads one bit (the `NEXTBIT` of Figure F.16).
    fn read_bit(&mut self) -> Result<i32> {
        self.ensure(1)?;
        self.bit_count -= 1;
        Ok(((self.bit_buf >> self.bit_count) & 1) as i32)
    }

    /// Whether, at an MCU-row boundary, only `1`-padding remains before a marker — the end of the
    /// entropy data for a `Y = 0` (DNL) frame whose MCU-row count is not known in advance. In the
    /// middle of a scan the pending bits are real data (or the next byte is not a marker), so this is
    /// `false`; a false positive would require the pending bits to be all-`1` *and* the next byte to
    /// begin a marker, which coincide only at the true end.
    fn at_data_end(&self) -> bool {
        if self.marker.is_some() {
            return true;
        }
        if self.bit_count >= 8 {
            return false; // a whole unconsumed data byte is buffered
        }
        let mask = (1u32 << self.bit_count) - 1;
        if (self.bit_buf & mask) != mask {
            return false; // pending bits are not all-1 padding
        }
        // Peek forward for a marker without consuming (mirrors `fetch`): skip any run of fill 0xFF
        // bytes (§B.1.1.2) and classify the byte that follows.
        let fill = self
            .data
            .get(self.pos..)
            .unwrap_or_default()
            .iter()
            .take_while(|&&b| b == 0xFF)
            .count();
        match self.data.get(self.pos + fill) {
            // No 0xFF prefix at all: still inside the destuffed data byte stream.
            _ if fill == 0 => false,
            // Truncated, or a stuffed `0xFF 0x00` (literal data), or a restart marker: not the end.
            // A restart (RSTm) is a mid-scan byte-aligned boundary — a complete scan never emits a
            // trailing RST, so an RSTm always precedes more MCUs (§E.2.5). Treating it as the end
            // would truncate a Y=0 frame whose restart interval lands on an MCU-row start.
            None | Some(0x00) => false,
            Some(&m) if (0xD0..=0xD7).contains(&m) => false,
            // Any other marker (EOI, DNL, …) terminates the entropy data.
            Some(_) => true,
        }
    }

    /// Consumes a restart marker at an interval boundary (§E.2.5): byte-aligns, verifies the marker
    /// is `RSTm` with `m == expected`, and advances past it. The caller resets the DC predictors.
    fn take_restart(&mut self, expected: u8) -> Result<()> {
        self.bit_count = 0;
        self.bit_buf = 0;
        let code = loop {
            match self.fetch() {
                Fetch::Data(_) => continue,
                Fetch::Marker => break self.marker.unwrap_or(0),
                Fetch::Eof => {
                    return Err(Error::InvalidInput("JPEG: truncated before restart marker"));
                }
            }
        };
        if !(0xD0..=0xD7).contains(&code) {
            return Err(Error::InvalidInput("JPEG: expected restart marker"));
        }
        if code - 0xD0 != expected {
            return Err(Error::InvalidInput("JPEG: restart marker out of sequence"));
        }
        self.pos = self.marker_off + 2;
        self.marker = None;
        Ok(())
    }

    /// Byte-aligns and advances to the marker that ends the scan, returning `(code, offset)` without
    /// consuming it (the segment parser resumes at `offset`).
    fn end_marker(&mut self) -> Result<(u8, usize)> {
        self.bit_count = 0;
        self.bit_buf = 0;
        loop {
            match self.fetch() {
                Fetch::Data(_) => continue,
                Fetch::Marker => return Ok((self.marker.unwrap_or(0), self.marker_off)),
                Fetch::Eof => return Err(Error::InvalidInput("JPEG: missing marker after scan")),
            }
        }
    }
}

/// `EXTEND` (Figure F.12): sign-extends the `t`-bit magnitude `v` to a signed value. `t ≥ 1`.
fn extend(v: u32, t: u8) -> i32 {
    let t = u32::from(t);
    let v = v as i32;
    if v < (1 << (t - 1)) {
        v - (1 << t) + 1
    } else {
        v
    }
}

/// `DECODE` (Figure F.16): reads bits until a canonical code matches, returning its symbol.
///
/// The code is grown with `+` rather than `|`: the shift just vacated the low bit and the addend is
/// a single bit, so the addition fills exactly that vacant bit.
fn decode_symbol(table: &DecTable, r: &mut BitReader) -> Result<u8> {
    let mut length = 1usize;
    let mut code = r.read_bit()?;
    while code > table.maxcode(length) {
        length += 1;
        if length > 16 {
            return Err(Error::InvalidInput("JPEG: undecodable Huffman code"));
        }
        code = (code << 1) + r.read_bit()?;
    }
    table
        .value_at(length, code)
        .ok_or(Error::InvalidInput("JPEG: Huffman value out of table"))
}

/// Per-scan-component decode context: the tables it uses, its sampling, and its running DC predictor
/// and output plane.
struct Ctx<'a> {
    frame_index: usize,
    h: u8,
    v: u8,
    dc: &'a DecTable,
    ac: &'a DecTable,
    quant: &'a [u16; 64],
    pred: i32,
    plane: Plane,
}

impl Ctx<'_> {
    /// Decodes one 8×8 block (§F.2.2): the differential DC coefficient then the run/size AC
    /// coefficients, dequantizes into natural order, inverse-transforms, level-shifts (+128) and
    /// clamps to `0..=255`, and writes the samples into the plane at block position `(bx, by)`
    /// (§A.3.1).
    fn decode_block(&mut self, r: &mut BitReader, bx: usize, by: usize) -> Result<()> {
        let mut zz = [0i32; 64];
        // DC: differential against the running predictor (§F.2.2.1).
        let t = decode_symbol(self.dc, r)?;
        if t > 11 {
            return Err(Error::InvalidInput("JPEG: DC magnitude category > 11"));
        }
        let diff = if t == 0 {
            0
        } else {
            extend(r.read_bits(u32::from(t))?, t)
        };
        self.pred = self.pred.wrapping_add(diff);
        zz[0] = self.pred.wrapping_mul(i32::from(self.quant[0]));
        // AC: run/size symbols in zig-zag order (§F.2.2.2).
        let mut k = 1usize;
        while k < 64 {
            let rs = decode_symbol(self.ac, r)?;
            let run = usize::from(rs >> 4);
            let size = rs & 0x0F;
            if size == 0 {
                if run == 15 {
                    k += 16; // ZRL: sixteen zero coefficients
                    continue;
                }
                break; // EOB: the rest of the block is zero
            }
            k += run;
            if k > 63 {
                return Err(Error::InvalidInput("JPEG: AC coefficient index past 63"));
            }
            let coeff = extend(r.read_bits(u32::from(size))?, size);
            let natural = ZIGZAG[k];
            zz[natural] = coeff.wrapping_mul(i32::from(self.quant[natural]));
            k += 1;
        }
        // Inverse DCT, level shift and clamp (§A.3.1).
        idct8x8(&mut zz);
        self.plane.ensure_rows((by + 1) * 8);
        let stride = self.plane.stride;
        for row in 0..8 {
            let dst = (by * 8 + row) * stride + bx * 8;
            for col in 0..8 {
                self.plane.data[dst + col] = (zz[row * 8 + col] + 128).clamp(0, 255) as u8;
            }
        }
        Ok(())
    }
}

/// Decodes the entropy data of one scan starting at byte `start`, per §A.2 and Annex F.
///
/// `frame` gives the component geometry (Hmax/Vmax, X/Y); `scan` selects the coded components and
/// their tables; `tables` holds the current dequantization/Huffman tables and restart interval. When
/// `frame.y == 0` the MCU-row count is unknown up front and rows are decoded until the entropy data
/// ends at a marker (a `Y = 0` frame's height arrives in the following DNL segment).
///
/// # Errors
///
/// [`Error::InvalidInput`] for an undefined referenced table, a corrupt Huffman code, an
/// out-of-range coefficient, a bad restart sequence, or a truncated stream.
pub fn decode_scan(
    data: &[u8],
    start: usize,
    frame: &Frame,
    scan: &ScanHeader,
    tables: &Tables,
) -> Result<ScanResult> {
    let hmax = usize::from(frame.hmax());
    let vmax = usize::from(frame.vmax());
    let x = usize::from(frame.x);
    let interleaved = scan.interleaved();

    // Horizontal MCU count is always known (X ≠ 0). The vertical count is known only when Y ≠ 0.
    let mcus_x = if interleaved {
        x.div_ceil(8 * hmax)
    } else {
        // Non-interleaved: MCU = one block; blocks span the component's own width (§A.2.2).
        let fc = &frame.components[scan.components[0].frame_index];
        let comp_w = (x * usize::from(fc.h)).div_ceil(hmax);
        comp_w.div_ceil(8)
    };
    let mcus_y: Option<usize> = if frame.y == 0 {
        None
    } else {
        let y = usize::from(frame.y);
        Some(if interleaved {
            y.div_ceil(8 * vmax)
        } else {
            let fc = &frame.components[scan.components[0].frame_index];
            let comp_h = (y * usize::from(fc.v)).div_ceil(vmax);
            comp_h.div_ceil(8)
        })
    };

    // Build a decode context per scan component, sized to its block layout in this scan.
    let mut ctxs: Vec<Ctx> = Vec::with_capacity(scan.components.len());
    for sc in &scan.components {
        let fc = &frame.components[sc.frame_index];
        let dc = tables.dc[usize::from(sc.td)]
            .as_ref()
            .ok_or(Error::InvalidInput("JPEG: scan uses undefined DC table"))?;
        let ac = tables.ac[usize::from(sc.ta)]
            .as_ref()
            .ok_or(Error::InvalidInput("JPEG: scan uses undefined AC table"))?;
        let quant = tables.quant[usize::from(fc.tq)]
            .as_ref()
            .ok_or(Error::InvalidInput("JPEG: scan uses undefined quant table"))?;
        let blk_cols = if interleaved { usize::from(fc.h) } else { 1 };
        let blocks_per_line = mcus_x * blk_cols;
        ctxs.push(Ctx {
            frame_index: sc.frame_index,
            h: fc.h,
            v: fc.v,
            dc,
            ac,
            quant,
            pred: 0,
            plane: Plane {
                data: Vec::new(),
                stride: blocks_per_line * 8,
                h: fc.h,
                v: fc.v,
            },
        });
    }

    let mut reader = BitReader::new(data, start);
    let restart = usize::from(tables.restart_interval);
    let mut mcus_done = 0usize;
    let mut expected_rst = 0u8;
    let mut mcu_y = 0usize;

    loop {
        match mcus_y {
            Some(rows) if mcu_y >= rows => break,
            None if reader.at_data_end() => break,
            _ => {}
        }
        for mcu_x in 0..mcus_x {
            if restart != 0 && mcus_done != 0 && mcus_done.is_multiple_of(restart) {
                reader.take_restart(expected_rst)?;
                expected_rst = (expected_rst + 1) & 7;
                for ctx in &mut ctxs {
                    ctx.pred = 0;
                }
            }
            for ctx in &mut ctxs {
                if interleaved {
                    let (h, v) = (usize::from(ctx.h), usize::from(ctx.v));
                    for by in 0..v {
                        for bx in 0..h {
                            ctx.decode_block(&mut reader, mcu_x * h + bx, mcu_y * v + by)?;
                        }
                    }
                } else {
                    ctx.decode_block(&mut reader, mcu_x, mcu_y)?;
                }
            }
            mcus_done += 1;
        }
        mcu_y += 1;
    }

    let (_marker, marker_offset) = reader.end_marker()?;
    let planes = ctxs.into_iter().map(|c| (c.frame_index, c.plane)).collect();
    Ok(ScanResult {
        planes,
        marker_offset,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extend_matches_figure_f12() {
        // t=1: v=0 → −1, v=1 → 1.
        assert_eq!(extend(0, 1), -1);
        assert_eq!(extend(1, 1), 1);
        // t=3: v in 0..3 map to −7..−4, v in 4..7 map to 4..7.
        assert_eq!(extend(0, 3), -7);
        assert_eq!(extend(3, 3), -4);
        assert_eq!(extend(4, 3), 4);
        assert_eq!(extend(7, 3), 7);
    }
}
