//! The multi-dimensional LUT transform element types (the crate's keystone): the legacy
//! `lut8Type`/`lut16Type` and the v4 `lutAToBType`/`lutBToAType` (ICC.1:2022 §10.10–10.13).
//!
//! These carry the matrix → curves → CLUT → curves pipeline that maps a device colour space to and
//! from the PCS. gamut-icc decodes their structure faithfully (it does not itself apply the
//! transform); raw lookup samples are preserved as integers so the elements round-trip exactly.

use gamut_core::{Error, Result};

use crate::bytes::{ByteReader, pad_to_4, push_s15fixed16};
use crate::curve::{CurveOrParametric, read_curve_element, write_curve_element};
use crate::primitives::S15Fixed16;

/// The sample precision of a CLUT.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClutPrecision {
    /// 8-bit samples (full scale 255).
    U8,
    /// 16-bit samples (full scale 65535).
    U16,
}

impl ClutPrecision {
    /// The full-scale value the samples are normalized against (255 or 65535).
    #[must_use]
    pub fn full_scale(self) -> u16 {
        match self {
            ClutPrecision::U8 => 255,
            ClutPrecision::U16 => 65535,
        }
    }
}

/// A colour lookup table: a regular grid of output samples indexed by the input channels
/// (`lutAToBType`/`lutBToAType` CLUT, ICC.1:2022 §10.12.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Clut {
    /// Grid points per input dimension (`len` == the transform's input-channel count).
    pub grid_points: Vec<u8>,
    /// Output channels produced per grid node.
    pub output_channels: u8,
    /// On-disk sample precision (re-emitted at the same width).
    pub precision: ClutPrecision,
    /// Output samples in grid order (last input channel varying fastest, then output channel),
    /// each a raw value in `0..=precision.full_scale()`.
    pub samples: Vec<u16>,
}

/// A row-major 3×3 matrix of `s15Fixed16` values (`lut8Type`/`lut16Type` matrix).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Matrix3x3 {
    /// The nine matrix elements, row-major.
    pub elements: [S15Fixed16; 9],
}

/// A 3×3 matrix with a 3-element offset (`lutAToBType`/`lutBToAType` matrix, ICC.1:2022 §10.12.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Matrix3x4 {
    /// The nine matrix elements, row-major.
    pub matrix: [S15Fixed16; 9],
    /// The three output offsets.
    pub offset: [S15Fixed16; 3],
}

/// A `lut8Type` element (`mft1`, ICC.1:2022 §10.10): matrix → input tables → CLUT → output tables,
/// with 8-bit tables and CLUT and a uniform grid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lut8 {
    /// Input channel count.
    pub input_channels: u8,
    /// Output channel count.
    pub output_channels: u8,
    /// Grid points per input dimension (uniform across dimensions).
    pub grid_points: u8,
    /// The 3×3 matrix (applies only when the input is PCS XYZ; otherwise the identity).
    pub matrix: Matrix3x3,
    /// Input tables, concatenated: `input_channels` tables of 256 entries each.
    pub input_table: Vec<u8>,
    /// CLUT samples: `grid_points^input_channels * output_channels` entries.
    pub clut: Vec<u8>,
    /// Output tables, concatenated: `output_channels` tables of 256 entries each.
    pub output_table: Vec<u8>,
}

/// A `lut16Type` element (`mft2`, ICC.1:2022 §10.11): like [`Lut8`] but with 16-bit tables and CLUT
/// and a variable per-table entry count.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lut16 {
    /// Input channel count.
    pub input_channels: u8,
    /// Output channel count.
    pub output_channels: u8,
    /// Grid points per input dimension (uniform across dimensions).
    pub grid_points: u8,
    /// The 3×3 matrix (applies only when the input is PCS XYZ; otherwise the identity).
    pub matrix: Matrix3x3,
    /// Entries per input table.
    pub input_table_entries: u16,
    /// Entries per output table.
    pub output_table_entries: u16,
    /// Input tables, concatenated: `input_channels * input_table_entries` entries.
    pub input_table: Vec<u16>,
    /// CLUT samples: `grid_points^input_channels * output_channels` entries.
    pub clut: Vec<u16>,
    /// Output tables, concatenated: `output_channels * output_table_entries` entries.
    pub output_table: Vec<u16>,
}

/// A `lutAToBType` element (`mAB `, ICC.1:2022 §10.12): the device→PCS pipeline
/// A-curves → CLUT → M-curves → matrix → B-curves. Every stage but the B-curves is optional.
#[derive(Debug, Clone, PartialEq)]
pub struct LutAToB {
    /// Input (device) channel count.
    pub input_channels: u8,
    /// Output (PCS) channel count.
    pub output_channels: u8,
    /// The input "A" curves (`input_channels` of them), if present.
    pub a_curves: Option<Vec<CurveOrParametric>>,
    /// The multi-dimensional lookup table, if present.
    pub clut: Option<Clut>,
    /// The "M" curves (`output_channels` of them), if present.
    pub m_curves: Option<Vec<CurveOrParametric>>,
    /// The 3×3-plus-offset matrix, if present.
    pub matrix: Option<Matrix3x4>,
    /// The output "B" curves (`output_channels` of them); always present.
    pub b_curves: Vec<CurveOrParametric>,
}

/// A `lutBToAType` element (`mBA `, ICC.1:2022 §10.13): the PCS→device pipeline
/// B-curves → matrix → M-curves → CLUT → A-curves. Every stage but the B-curves is optional.
#[derive(Debug, Clone, PartialEq)]
pub struct LutBToA {
    /// Input (PCS) channel count.
    pub input_channels: u8,
    /// Output (device) channel count.
    pub output_channels: u8,
    /// The input "B" curves (`input_channels` of them); always present.
    pub b_curves: Vec<CurveOrParametric>,
    /// The 3×3-plus-offset matrix, if present.
    pub matrix: Option<Matrix3x4>,
    /// The "M" curves (`input_channels` of them), if present.
    pub m_curves: Option<Vec<CurveOrParametric>>,
    /// The multi-dimensional lookup table, if present.
    pub clut: Option<Clut>,
    /// The output "A" curves (`output_channels` of them), if present.
    pub a_curves: Option<Vec<CurveOrParametric>>,
}

/// Decodes a `lut8Type` element.
pub(crate) fn decode_lut8(element: &[u8]) -> Result<Lut8> {
    let mut r = ByteReader::at(element, 8)?;
    let input_channels = r.u8()?;
    let output_channels = r.u8()?;
    let grid_points = r.u8()?;
    r.skip(1)?; // reserved
    let matrix = read_matrix3x3(&mut r)?;
    let input_table = r.bytes(table_len(input_channels, 256)?)?.to_vec();
    let clut = r
        .bytes(clut_len(grid_points, input_channels, output_channels)?)?
        .to_vec();
    let output_table = r.bytes(table_len(output_channels, 256)?)?.to_vec();
    Ok(Lut8 {
        input_channels,
        output_channels,
        grid_points,
        matrix,
        input_table,
        clut,
        output_table,
    })
}

/// Decodes a `lut16Type` element.
pub(crate) fn decode_lut16(element: &[u8]) -> Result<Lut16> {
    let mut r = ByteReader::at(element, 8)?;
    let input_channels = r.u8()?;
    let output_channels = r.u8()?;
    let grid_points = r.u8()?;
    r.skip(1)?; // reserved
    let matrix = read_matrix3x3(&mut r)?;
    let input_table_entries = r.u16()?;
    let output_table_entries = r.u16()?;
    let input_table = read_u16_vec(
        &mut r,
        table_len(input_channels, input_table_entries.into())?,
    )?;
    let clut = read_u16_vec(
        &mut r,
        clut_len(grid_points, input_channels, output_channels)?,
    )?;
    let output_table = read_u16_vec(
        &mut r,
        table_len(output_channels, output_table_entries.into())?,
    )?;
    Ok(Lut16 {
        input_channels,
        output_channels,
        grid_points,
        matrix,
        input_table_entries,
        output_table_entries,
        input_table,
        clut,
        output_table,
    })
}

/// Decodes a `lutAToBType` element.
pub(crate) fn decode_lut_a_to_b(element: &[u8]) -> Result<LutAToB> {
    let (input_channels, output_channels, offsets) = read_mab_header(element)?;
    let [off_b, off_matrix, off_m, off_clut, off_a] = offsets;
    let i = input_channels as usize;
    let o = output_channels as usize;
    if off_b == 0 {
        return Err(Error::InvalidInput("icc: lutAToB missing B-curves"));
    }
    Ok(LutAToB {
        input_channels,
        output_channels,
        a_curves: read_optional_curves(element, off_a, i)?,
        clut: read_optional_clut(element, off_clut, input_channels, output_channels)?,
        m_curves: read_optional_curves(element, off_m, o)?,
        matrix: read_optional_matrix(element, off_matrix)?,
        b_curves: read_curves(element, off_b, o)?,
    })
}

/// Decodes a `lutBToAType` element.
pub(crate) fn decode_lut_b_to_a(element: &[u8]) -> Result<LutBToA> {
    let (input_channels, output_channels, offsets) = read_mab_header(element)?;
    let [off_b, off_matrix, off_m, off_clut, off_a] = offsets;
    let i = input_channels as usize;
    let o = output_channels as usize;
    if off_b == 0 {
        return Err(Error::InvalidInput("icc: lutBToA missing B-curves"));
    }
    Ok(LutBToA {
        input_channels,
        output_channels,
        b_curves: read_curves(element, off_b, i)?,
        matrix: read_optional_matrix(element, off_matrix)?,
        m_curves: read_optional_curves(element, off_m, i)?,
        clut: read_optional_clut(element, off_clut, input_channels, output_channels)?,
        a_curves: read_optional_curves(element, off_a, o)?,
    })
}

/// Reads the shared `mAB `/`mBA ` header: input/output channels and the five stage offsets
/// (B-curves, matrix, M-curves, CLUT, A-curves), each relative to the element start.
fn read_mab_header(element: &[u8]) -> Result<(u8, u8, [usize; 5])> {
    let mut r = ByteReader::at(element, 8)?;
    let input_channels = r.u8()?;
    let output_channels = r.u8()?;
    r.skip(2)?; // reserved
    let offsets = [
        r.u32()? as usize,
        r.u32()? as usize,
        r.u32()? as usize,
        r.u32()? as usize,
        r.u32()? as usize,
    ];
    Ok((input_channels, output_channels, offsets))
}

fn read_matrix3x3(r: &mut ByteReader<'_>) -> Result<Matrix3x3> {
    let mut elements = [S15Fixed16(0); 9];
    for element in &mut elements {
        *element = r.s15fixed16()?;
    }
    Ok(Matrix3x3 { elements })
}

fn read_optional_matrix(element: &[u8], offset: usize) -> Result<Option<Matrix3x4>> {
    if offset == 0 {
        return Ok(None);
    }
    let mut r = ByteReader::at(element, offset)?;
    let mut matrix = [S15Fixed16(0); 9];
    for m in &mut matrix {
        *m = r.s15fixed16()?;
    }
    let mut off = [S15Fixed16(0); 3];
    for o in &mut off {
        *o = r.s15fixed16()?;
    }
    Ok(Some(Matrix3x4 {
        matrix,
        offset: off,
    }))
}

fn read_curves(element: &[u8], offset: usize, count: usize) -> Result<Vec<CurveOrParametric>> {
    let mut r = ByteReader::at(element, offset)?;
    let mut curves = Vec::with_capacity(count);
    for _ in 0..count {
        curves.push(read_curve_element(&mut r)?);
    }
    Ok(curves)
}

fn read_optional_curves(
    element: &[u8],
    offset: usize,
    count: usize,
) -> Result<Option<Vec<CurveOrParametric>>> {
    if offset == 0 {
        Ok(None)
    } else {
        Ok(Some(read_curves(element, offset, count)?))
    }
}

fn read_optional_clut(
    element: &[u8],
    offset: usize,
    input_channels: u8,
    output_channels: u8,
) -> Result<Option<Clut>> {
    if offset == 0 {
        return Ok(None);
    }
    if input_channels as usize > 16 {
        return Err(Error::InvalidInput("icc: CLUT input channels exceed 16"));
    }
    let mut r = ByteReader::at(element, offset)?;
    let grid_points = r.bytes(16)?[..input_channels as usize].to_vec();
    let precision = match r.u8()? {
        1 => ClutPrecision::U8,
        2 => ClutPrecision::U16,
        _ => return Err(Error::InvalidInput("icc: invalid CLUT precision")),
    };
    r.skip(3)?; // reserved
    let sample_count = grid_node_count(&grid_points)?
        .checked_mul(output_channels as usize)
        .ok_or(Error::InvalidInput("icc: CLUT sample count overflow"))?;
    let samples = match precision {
        ClutPrecision::U8 => r
            .bytes(sample_count)?
            .iter()
            .map(|&b| u16::from(b))
            .collect(),
        ClutPrecision::U16 => read_u16_vec(&mut r, sample_count)?,
    };
    Ok(Some(Clut {
        grid_points,
        output_channels,
        precision,
        samples,
    }))
}

/// `channels * entries`, checked.
fn table_len(channels: u8, entries: usize) -> Result<usize> {
    (channels as usize)
        .checked_mul(entries)
        .ok_or(Error::InvalidInput("icc: LUT table size overflow"))
}

/// `grid_points^input_channels * output_channels`, checked.
fn clut_len(grid_points: u8, input_channels: u8, output_channels: u8) -> Result<usize> {
    grid_node_count(&vec![grid_points; input_channels as usize])?
        .checked_mul(output_channels as usize)
        .ok_or(Error::InvalidInput("icc: CLUT size overflow"))
}

/// The product of the per-dimension grid points (the number of CLUT nodes), checked.
fn grid_node_count(grid_points: &[u8]) -> Result<usize> {
    let mut nodes = 1usize;
    for &g in grid_points {
        nodes = nodes
            .checked_mul(g as usize)
            .ok_or(Error::InvalidInput("icc: CLUT grid overflow"))?;
    }
    Ok(nodes)
}

fn read_u16_vec(r: &mut ByteReader<'_>, count: usize) -> Result<Vec<u16>> {
    let byte_len = count
        .checked_mul(2)
        .ok_or(Error::InvalidInput("icc: LUT size overflow"))?;
    let bytes = r.bytes(byte_len)?;
    Ok(bytes
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect())
}

/// Writes a `lut8Type` element — the inverse of [`decode_lut8`].
pub(crate) fn encode_lut8(lut: &Lut8, out: &mut Vec<u8>) {
    out.extend_from_slice(b"mft1");
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&[lut.input_channels, lut.output_channels, lut.grid_points, 0]);
    write_matrix3x3(&lut.matrix, out);
    out.extend_from_slice(&lut.input_table);
    out.extend_from_slice(&lut.clut);
    out.extend_from_slice(&lut.output_table);
}

/// Writes a `lut16Type` element — the inverse of [`decode_lut16`].
pub(crate) fn encode_lut16(lut: &Lut16, out: &mut Vec<u8>) {
    out.extend_from_slice(b"mft2");
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&[lut.input_channels, lut.output_channels, lut.grid_points, 0]);
    write_matrix3x3(&lut.matrix, out);
    out.extend_from_slice(&lut.input_table_entries.to_be_bytes());
    out.extend_from_slice(&lut.output_table_entries.to_be_bytes());
    write_u16_slice(&lut.input_table, out);
    write_u16_slice(&lut.clut, out);
    write_u16_slice(&lut.output_table, out);
}

/// Writes a `lutAToBType` element — the inverse of [`decode_lut_a_to_b`].
pub(crate) fn encode_lut_a_to_b(lut: &LutAToB, out: &mut Vec<u8>) {
    let mut body = Vec::new();
    let off_b = stage_offset(&body);
    for curve in &lut.b_curves {
        write_curve_element(curve, &mut body);
    }
    let off_matrix = write_optional_matrix(lut.matrix.as_ref(), &mut body);
    let off_m = write_optional_curves(lut.m_curves.as_ref(), &mut body);
    let off_clut = write_optional_clut(lut.clut.as_ref(), &mut body);
    let off_a = write_optional_curves(lut.a_curves.as_ref(), &mut body);
    write_mab(
        b"mAB ",
        lut.input_channels,
        lut.output_channels,
        [off_b, off_matrix, off_m, off_clut, off_a],
        &body,
        out,
    );
}

/// Writes a `lutBToAType` element — the inverse of [`decode_lut_b_to_a`].
pub(crate) fn encode_lut_b_to_a(lut: &LutBToA, out: &mut Vec<u8>) {
    let mut body = Vec::new();
    let off_b = stage_offset(&body);
    for curve in &lut.b_curves {
        write_curve_element(curve, &mut body);
    }
    let off_matrix = write_optional_matrix(lut.matrix.as_ref(), &mut body);
    let off_m = write_optional_curves(lut.m_curves.as_ref(), &mut body);
    let off_clut = write_optional_clut(lut.clut.as_ref(), &mut body);
    let off_a = write_optional_curves(lut.a_curves.as_ref(), &mut body);
    write_mab(
        b"mBA ",
        lut.input_channels,
        lut.output_channels,
        [off_b, off_matrix, off_m, off_clut, off_a],
        &body,
        out,
    );
}

/// The byte length of the shared `mAB `/`mBA ` header (type+reserved, channels+reserved, five
/// offsets); the offset a body stage lands at, relative to the element start.
const MAB_HEADER_LEN: usize = 8 + 4 + 5 * 4;

fn stage_offset(body: &[u8]) -> u32 {
    (MAB_HEADER_LEN + body.len()) as u32
}

fn write_mab(
    type_sig: &[u8; 4],
    input_channels: u8,
    output_channels: u8,
    offsets: [u32; 5],
    body: &[u8],
    out: &mut Vec<u8>,
) {
    out.extend_from_slice(type_sig);
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&[input_channels, output_channels, 0, 0]);
    for offset in offsets {
        out.extend_from_slice(&offset.to_be_bytes());
    }
    out.extend_from_slice(body);
}

fn write_optional_matrix(matrix: Option<&Matrix3x4>, body: &mut Vec<u8>) -> u32 {
    let Some(matrix) = matrix else { return 0 };
    let offset = stage_offset(body);
    for &m in &matrix.matrix {
        push_s15fixed16(body, m);
    }
    for &o in &matrix.offset {
        push_s15fixed16(body, o);
    }
    pad_to_4(body);
    offset
}

fn write_optional_curves(curves: Option<&Vec<CurveOrParametric>>, body: &mut Vec<u8>) -> u32 {
    let Some(curves) = curves else { return 0 };
    let offset = stage_offset(body);
    for curve in curves {
        write_curve_element(curve, body);
    }
    offset
}

fn write_optional_clut(clut: Option<&Clut>, body: &mut Vec<u8>) -> u32 {
    let Some(clut) = clut else { return 0 };
    let offset = stage_offset(body);
    let mut grid = [0u8; 16];
    let n = clut.grid_points.len().min(16);
    grid[..n].copy_from_slice(&clut.grid_points[..n]);
    body.extend_from_slice(&grid);
    body.push(match clut.precision {
        ClutPrecision::U8 => 1,
        ClutPrecision::U16 => 2,
    });
    body.extend_from_slice(&[0, 0, 0]); // reserved
    match clut.precision {
        ClutPrecision::U8 => {
            for &s in &clut.samples {
                body.push(s as u8);
            }
        }
        ClutPrecision::U16 => write_u16_slice(&clut.samples, body),
    }
    pad_to_4(body);
    offset
}

fn write_matrix3x3(matrix: &Matrix3x3, out: &mut Vec<u8>) {
    for &element in &matrix.elements {
        push_s15fixed16(out, element);
    }
}

fn write_u16_slice(values: &[u16], out: &mut Vec<u8>) {
    for &value in values {
        out.extend_from_slice(&value.to_be_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curve::{Curve, ParametricCurve};
    use crate::primitives::U8Fixed8;

    fn s15_be(v: f64) -> [u8; 4] {
        S15Fixed16::from_f64(v).0.to_be_bytes()
    }

    fn gamma_curve() -> CurveOrParametric {
        CurveOrParametric::Curve(Curve::Gamma(U8Fixed8(0x0200)))
    }

    #[test]
    fn decodes_lut8() {
        // 1 input channel, 1 output channel, 2 grid points.
        let mut e = b"mft1\x00\x00\x00\x00".to_vec();
        e.extend_from_slice(&[1, 1, 2, 0]); // i, o, grid, reserved
        for _ in 0..9 {
            e.extend_from_slice(&s15_be(0.0)); // matrix (identity content irrelevant here)
        }
        e.extend_from_slice(&[7u8; 256]); // input table (1 × 256)
        e.extend_from_slice(&[10, 20]); // CLUT (2^1 nodes × 1 output)
        e.extend_from_slice(&[9u8; 256]); // output table (1 × 256)

        let lut = decode_lut8(&e).unwrap();
        assert_eq!(lut.input_channels, 1);
        assert_eq!(lut.output_channels, 1);
        assert_eq!(lut.grid_points, 2);
        assert_eq!(lut.input_table.len(), 256);
        assert_eq!(lut.clut, vec![10, 20]);
        assert_eq!(lut.output_table.len(), 256);
    }

    #[test]
    fn decodes_lut16_clut_dimensions() {
        // 2 input channels, 3 output channels, 2 grid points → 2^2 × 3 = 12 CLUT samples.
        let mut e = b"mft2\x00\x00\x00\x00".to_vec();
        e.extend_from_slice(&[2, 3, 2, 0]);
        for _ in 0..9 {
            e.extend_from_slice(&s15_be(0.0));
        }
        e.extend_from_slice(&2u16.to_be_bytes()); // input table entries
        e.extend_from_slice(&2u16.to_be_bytes()); // output table entries
        for _ in 0..(2 * 2) {
            e.extend_from_slice(&0u16.to_be_bytes()); // input tables
        }
        for v in 0..12u16 {
            e.extend_from_slice(&v.to_be_bytes()); // CLUT
        }
        for _ in 0..(3 * 2) {
            e.extend_from_slice(&0u16.to_be_bytes()); // output tables
        }

        let lut = decode_lut16(&e).unwrap();
        assert_eq!(lut.input_channels, 2);
        assert_eq!(lut.output_channels, 3);
        assert_eq!(lut.clut.len(), 12);
        assert_eq!(lut.clut[11], 11);
    }

    #[test]
    fn decodes_lut_a_to_b_minimal() {
        // 3→3 with only the (required) B-curves: three identity curveType curves.
        let header_len = 8 + 4 + 5 * 4; // type+reserved, channels+reserved, five offsets
        let mut e = b"mAB \x00\x00\x00\x00".to_vec();
        e.extend_from_slice(&[3, 3, 0, 0]); // i, o, reserved
        e.extend_from_slice(&(header_len as u32).to_be_bytes()); // B-curves offset
        e.extend_from_slice(&0u32.to_be_bytes()); // matrix
        e.extend_from_slice(&0u32.to_be_bytes()); // M-curves
        e.extend_from_slice(&0u32.to_be_bytes()); // CLUT
        e.extend_from_slice(&0u32.to_be_bytes()); // A-curves
        for _ in 0..3 {
            e.extend_from_slice(b"curv\x00\x00\x00\x00"); // identity curve element
            e.extend_from_slice(&0u32.to_be_bytes()); // count 0
        }

        let lut = decode_lut_a_to_b(&e).unwrap();
        assert_eq!(lut.input_channels, 3);
        assert_eq!(lut.output_channels, 3);
        assert_eq!(lut.b_curves.len(), 3);
        assert!(matches!(
            lut.b_curves[0],
            CurveOrParametric::Curve(Curve::Identity)
        ));
        assert!(lut.a_curves.is_none());
        assert!(lut.clut.is_none());
        assert!(lut.m_curves.is_none());
        assert!(lut.matrix.is_none());
    }

    #[test]
    fn decodes_lut_a_to_b_with_clut_and_matrix() {
        // 3→3 with B-curves, a matrix, and a CLUT (2 grid points per axis, u8 precision).
        let mut e = b"mAB \x00\x00\x00\x00".to_vec();
        e.extend_from_slice(&[3, 3, 0, 0]);
        // Offsets filled in after laying out the body.
        let offsets_at = e.len();
        e.extend_from_slice(&[0u8; 20]);

        let off_b = e.len();
        for _ in 0..3 {
            e.extend_from_slice(b"curv\x00\x00\x00\x00");
            e.extend_from_slice(&0u32.to_be_bytes());
        }
        let off_matrix = e.len();
        for v in [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.1, 0.2, 0.3] {
            e.extend_from_slice(&s15_be(v));
        }
        let off_clut = e.len();
        e.extend_from_slice(&[2u8; 16]); // grid points per dimension
        e.push(1); // precision: u8
        e.extend_from_slice(&[0, 0, 0]); // reserved
        // 2^3 nodes × 3 outputs = 24 samples.
        e.extend_from_slice(&(0..24u8).collect::<Vec<_>>());

        let patch = |e: &mut Vec<u8>, at: usize, v: u32| {
            e[at..at + 4].copy_from_slice(&v.to_be_bytes());
        };
        patch(&mut e, offsets_at, off_b as u32);
        patch(&mut e, offsets_at + 4, off_matrix as u32);
        patch(&mut e, offsets_at + 12, off_clut as u32);

        let lut = decode_lut_a_to_b(&e).unwrap();
        let clut = lut.clut.expect("clut present");
        assert_eq!(clut.grid_points, vec![2, 2, 2]);
        assert_eq!(clut.output_channels, 3);
        assert_eq!(clut.precision, ClutPrecision::U8);
        assert_eq!(clut.samples.len(), 24);
        let matrix = lut.matrix.expect("matrix present");
        assert_eq!(matrix.offset[0], S15Fixed16::from_f64(0.1));
    }

    #[test]
    fn lut8_round_trips() {
        let lut = Lut8 {
            input_channels: 1,
            output_channels: 1,
            grid_points: 2,
            matrix: Matrix3x3 {
                elements: [S15Fixed16::from_f64(1.0); 9],
            },
            input_table: vec![7u8; 256],
            clut: vec![10, 20],
            output_table: vec![9u8; 256],
        };
        let mut out = Vec::new();
        encode_lut8(&lut, &mut out);
        assert_eq!(decode_lut8(&out).unwrap(), lut);
    }

    #[test]
    fn lut16_round_trips() {
        let lut = Lut16 {
            input_channels: 2,
            output_channels: 3,
            grid_points: 2,
            matrix: Matrix3x3 {
                elements: [S15Fixed16(0); 9],
            },
            input_table_entries: 2,
            output_table_entries: 2,
            input_table: vec![0u16; 4],
            clut: (0..12u16).collect(),
            output_table: vec![0u16; 6],
        };
        let mut out = Vec::new();
        encode_lut16(&lut, &mut out);
        assert_eq!(decode_lut16(&out).unwrap(), lut);
    }

    #[test]
    fn lut_a_to_b_round_trips_every_stage() {
        // All five stages present, with a 16-bit CLUT.
        let lut = LutAToB {
            input_channels: 3,
            output_channels: 3,
            a_curves: Some(vec![gamma_curve(), gamma_curve(), gamma_curve()]),
            clut: Some(Clut {
                grid_points: vec![2, 2, 2],
                output_channels: 3,
                precision: ClutPrecision::U16,
                samples: (0..24u16).collect(),
            }),
            m_curves: Some(vec![gamma_curve(), gamma_curve(), gamma_curve()]),
            matrix: Some(Matrix3x4 {
                matrix: [S15Fixed16::from_f64(1.0); 9],
                offset: [S15Fixed16::from_f64(0.5); 3],
            }),
            b_curves: vec![gamma_curve(), gamma_curve(), gamma_curve()],
        };
        let mut out = Vec::new();
        encode_lut_a_to_b(&lut, &mut out);
        assert_eq!(decode_lut_a_to_b(&out).unwrap(), lut);
    }

    #[test]
    fn lut_b_to_a_round_trips_every_stage() {
        // 3 input (PCS) → 4 output (device) channels; an 8-bit CLUT and parametric curves.
        let param = || {
            CurveOrParametric::Parametric(ParametricCurve {
                function_type: 0,
                params: vec![S15Fixed16::from_f64(2.0)],
            })
        };
        let lut = LutBToA {
            input_channels: 3,
            output_channels: 4,
            b_curves: vec![param(), param(), param()],
            matrix: Some(Matrix3x4 {
                matrix: [S15Fixed16(0); 9],
                offset: [S15Fixed16(0); 3],
            }),
            m_curves: Some(vec![param(), param(), param()]),
            clut: Some(Clut {
                grid_points: vec![2, 2, 2],
                output_channels: 4,
                precision: ClutPrecision::U8,
                samples: (0..32u16).collect(), // 2^3 nodes × 4 outputs
            }),
            a_curves: Some(vec![param(), param(), param(), param()]),
        };
        let mut out = Vec::new();
        encode_lut_b_to_a(&lut, &mut out);
        assert_eq!(decode_lut_b_to_a(&out).unwrap(), lut);
    }

    #[test]
    fn rejects_invalid_clut_precision() {
        // An mAB whose CLUT precision byte is neither 1 nor 2.
        let mut e = b"mAB \x00\x00\x00\x00".to_vec();
        e.extend_from_slice(&[3, 3, 0, 0]);
        let offsets_at = e.len();
        e.extend_from_slice(&[0u8; 20]);
        let off_b = e.len();
        for _ in 0..3 {
            e.extend_from_slice(b"curv\x00\x00\x00\x00");
            e.extend_from_slice(&0u32.to_be_bytes());
        }
        let off_clut = e.len();
        e.extend_from_slice(&[2u8; 16]);
        e.push(9); // invalid precision
        e.extend_from_slice(&[0, 0, 0]);
        e.extend_from_slice(&[0u8; 24]);
        e[offsets_at..offsets_at + 4].copy_from_slice(&(off_b as u32).to_be_bytes());
        e[offsets_at + 12..offsets_at + 16].copy_from_slice(&(off_clut as u32).to_be_bytes());
        assert!(decode_lut_a_to_b(&e).is_err());
    }
}
