//! The sequential DCT Huffman decoder: [`JpegDecoder`] and the marker-loop driver.
//!
//! Decoding is where the codec is generous (T.81 Annex F §F.2 and Annex A §A.2): it accepts any
//! spec-valid baseline (SOF0) or extended-sequential (SOF1) 8-bit stream, resolves the colour space
//! from the JFIF/Adobe application segments, and rejects malformed input with a typed [`Error`]
//! rather than panicking. The pipeline is: walk the marker segments (SOI → tables → frame → scans →
//! EOI, [`crate::syntax`]); decode each scan's entropy data into per-component sample planes
//! ([`crate::scan`]); upsample the chroma planes to full resolution by sample replication; and
//! colour-convert to the requested pixel layout.
//!
//! # Colour interpretation
//!
//! Following the de-facto rules that libjpeg-family decoders use, the component count and the
//! advisory APP0/APP14 segments select the transform:
//!
//! - **1 component** → grayscale.
//! - **3 components** → Adobe APP14 `transform=1` ⇒ YCbCr, `transform=0` ⇒ RGB; with no APP14, a
//!   JFIF APP0 ⇒ YCbCr; with neither, component ids `R,G,B` (`0x52,0x47,0x42`) ⇒ RGB, otherwise
//!   YCbCr (the de-facto default).
//! - **4 components** → Adobe APP14 `transform=2` ⇒ YCCK (the inverse YCbCr transform on the first
//!   three channels, `K` passed through); otherwise CMYK stored verbatim (no Adobe inversion, the
//!   libjpeg convention).
//! - **2 or > 4 components** → [`Error::Unsupported`].
//!
//! # Upsampling
//!
//! Chroma planes coded at reduced resolution are upsampled to full resolution by **sample
//! replication** (nearest-neighbour). T.81 leaves the reconstruction filter open (§A.2 NOTE); sample
//! replication is the decoder's documented free choice — exact for 4:4:4 and the cheapest faithful
//! choice for subsampled chroma.

use gamut_color::{ColorRange, ycbcr_to_rgb};
use gamut_core::{Cmyk8, DecodeImage, Dimensions, Error, Gray8, ImageBuf, Result, Rgb8};

use crate::marker::code;
use crate::scan::{Plane, decode_scan};
use crate::syntax::{
    ColorInfo, Frame, Tables, parse_app0, parse_app14, parse_dht, parse_dnl, parse_dqt, parse_dri,
    parse_sof, parse_sos,
};

/// A decoder for sequential (baseline SOF0 / extended SOF1) 8-bit Huffman JPEG streams.
///
/// Stateless and cheap to construct; drive it through the [`DecodeImage`] trait. It presents the
/// decoded image as [`Rgb8`] (grayscale replicated, YCbCr/RGB three-component), [`Gray8`]
/// (single-component streams only), or [`Cmyk8`] (four-component CMYK/YCCK streams only).
///
/// # Example
///
/// ```
/// use gamut_core::{DecodeImage, EncodeImage, Dimensions, Gray8, ImageRef, Rgb8};
/// use gamut_jpeg::{JpegDecoder, JpegEncoder};
///
/// // Round-trip a small grayscale image through the encoder and back.
/// let pixels: Vec<u8> = (0..64).map(|i| (i * 4) as u8).collect();
/// let image = ImageRef::<Gray8>::new(&pixels, Dimensions::new(8, 8)?)?;
/// let jpeg = JpegEncoder::new().with_quality(90).encode_to_vec(image)?;
///
/// let decoded = JpegDecoder::new().decode_image(&jpeg)?;
/// let _rgb: gamut_core::ImageBuf<Rgb8> = decoded; // grayscale replicated across channels
/// # Ok::<(), gamut_core::Error>(())
/// ```
#[derive(Debug, Clone, Default)]
pub struct JpegDecoder {
    _private: (),
}

impl JpegDecoder {
    /// Creates a decoder.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// The DCT process a JPEG stream uses, as reported by [`info`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum JpegProcess {
    /// Baseline sequential DCT (SOF0).
    Baseline,
    /// Extended sequential DCT (SOF1).
    ExtendedSequential,
    /// Progressive DCT (SOF2).
    Progressive,
}

/// A lightweight summary of a JPEG stream's frame header, from [`info`].
///
/// Reads only the marker segments up to and including the frame header — no entropy decoding — so it
/// is cheap to call before committing to a full decode. Marked `#[non_exhaustive]` so fields can be
/// added without a breaking change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct JpegInfo {
    /// Samples per line `X` (§B.2.2).
    pub width: u32,
    /// Number of lines `Y`; `0` if the frame defers its height to a DNL segment.
    pub height: u32,
    /// Number of components `Nf`.
    pub components: u8,
    /// Sample precision `P` in bits.
    pub precision: u8,
    /// The DCT process (from the SOFn marker).
    pub process: JpegProcess,
}

/// Reads a JPEG stream's frame header without decoding it, returning its [`JpegInfo`].
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if the stream is malformed or has no frame header, or
/// [`Error::Unsupported`] if the frame uses a process other than baseline / extended-sequential /
/// progressive DCT.
pub fn info(data: &[u8]) -> Result<JpegInfo> {
    expect_soi(data)?;
    let mut pos = 2;
    loop {
        let (marker, after) = read_marker(data, pos)?;
        let process = match marker {
            code::SOF0 => JpegProcess::Baseline,
            code::SOF1 => JpegProcess::ExtendedSequential,
            code::SOF2 => JpegProcess::Progressive,
            code::SOF3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF => {
                return Err(Error::Unsupported("JPEG: unsupported process"));
            }
            code::SOS | code::EOI_CODE => {
                return Err(Error::InvalidInput("JPEG: no frame header before scan/end"));
            }
            code::SOI | code::TEM | code::RST0..=code::RST7 => {
                return Err(Error::InvalidInput("JPEG: unexpected standalone marker"));
            }
            _ => {
                // Any other segment (tables, application data, comment): skip by length.
                let (_, next) = read_segment(data, after)?;
                pos = next;
                continue;
            }
        };
        let (payload, _) = read_segment(data, after)?;
        // Frame header layout (§B.2.2): P, Y(2), X(2), Nf, then components.
        let precision = *payload.first().ok_or(TRUNC_SOF)?;
        let y = u16::from_be_bytes([
            *payload.get(1).ok_or(TRUNC_SOF)?,
            *payload.get(2).ok_or(TRUNC_SOF)?,
        ]);
        let x = u16::from_be_bytes([
            *payload.get(3).ok_or(TRUNC_SOF)?,
            *payload.get(4).ok_or(TRUNC_SOF)?,
        ]);
        let nf = *payload.get(5).ok_or(TRUNC_SOF)?;
        return Ok(JpegInfo {
            width: u32::from(x),
            height: u32::from(y),
            components: nf,
            precision,
            process,
        });
    }
}

/// Shared truncated-frame-header error.
const TRUNC_SOF: Error = Error::InvalidInput("JPEG: truncated frame header");

/// Verifies the two-byte SOI that opens every JPEG stream (§B.2.1).
fn expect_soi(data: &[u8]) -> Result<()> {
    if data.len() < 2 || data[0] != 0xFF || data[1] != code::SOI {
        return Err(Error::InvalidInput("JPEG: missing SOI marker"));
    }
    Ok(())
}

/// Reads the next marker at or after `pos` (skipping fill `0xFF` bytes, §B.1.1.2), returning its
/// code and the offset just past it.
fn read_marker(data: &[u8], pos: usize) -> Result<(u8, usize)> {
    if data.get(pos) != Some(&0xFF) {
        return Err(Error::InvalidInput("JPEG: expected a marker"));
    }
    // The marker code is the first non-fill byte: skip any run of fill 0xFF bytes (§B.1.1.2).
    let code_pos = pos + data[pos..].iter().take_while(|&&b| b == 0xFF).count();
    let code = *data
        .get(code_pos)
        .ok_or(Error::InvalidInput("JPEG: truncated marker"))?;
    Ok((code, code_pos + 1))
}

/// Reads a marker segment's payload at `pos` (the offset just past the marker code), returning the
/// payload bytes and the offset just past the segment. The two-byte length counts itself (§B.1.1.4).
fn read_segment(data: &[u8], pos: usize) -> Result<(&[u8], usize)> {
    let hi = *data.get(pos).ok_or(TRUNC_SEG)?;
    let lo = *data.get(pos + 1).ok_or(TRUNC_SEG)?;
    let len = usize::from(u16::from_be_bytes([hi, lo]));
    if len < 2 {
        return Err(Error::InvalidInput("JPEG: segment length < 2"));
    }
    let end = pos + len;
    let payload = data.get(pos + 2..end).ok_or(TRUNC_SEG)?;
    Ok((payload, end))
}

/// Shared truncated-segment error.
const TRUNC_SEG: Error = Error::InvalidInput("JPEG: truncated segment");

/// One decoded component ready for presentation: its sampling factors, the valid image region, and
/// the reconstructed plane at block-padded resolution (`stride` wide).
struct DecComp {
    h: u8,
    v: u8,
    comp_w: usize,
    comp_h: usize,
    stride: usize,
    data: Vec<u8>,
}

/// A fully decoded image: the reconstructed component planes plus the colour hints needed to present
/// them.
struct DecodedImage {
    width: u32,
    height: u32,
    hmax: usize,
    vmax: usize,
    comps: Vec<DecComp>,
    ids: Vec<u8>,
    color: ColorInfo,
}

impl DecodedImage {
    /// The sample of component `ci` under the output pixel `(px, py)`, via nearest-neighbour
    /// (sample-replication) upsampling.
    fn sample(&self, ci: usize, px: usize, py: usize) -> u8 {
        let c = &self.comps[ci];
        let cx = (px * usize::from(c.h) / self.hmax).min(c.comp_w - 1);
        let cy = (py * usize::from(c.v) / self.vmax).min(c.comp_h - 1);
        c.data[cy * c.stride + cx]
    }
}

/// The colour transform selected for presentation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transform {
    /// One luminance channel.
    Gray,
    /// Three channels stored directly as R, G, B.
    Rgb,
    /// Three channels YCbCr → RGB (T.871 §7).
    YCbCr,
    /// Four channels stored directly as C, M, Y, K.
    Cmyk,
    /// Four channels YCCK: inverse YCbCr on the first three, `K` passed through.
    Ycck,
}

/// The colour-interpretation decision tree (see the module docs).
fn decide_transform(img: &DecodedImage) -> Result<Transform> {
    match img.comps.len() {
        1 => Ok(Transform::Gray),
        3 => Ok(match img.color.adobe_transform {
            Some(0) => Transform::Rgb,
            Some(_) => Transform::YCbCr, // 1 = YCbCr; any other Adobe value defaults to YCbCr
            None => {
                if img.color.jfif {
                    Transform::YCbCr
                } else if img.ids.as_slice() == b"RGB" {
                    Transform::Rgb
                } else {
                    Transform::YCbCr
                }
            }
        }),
        4 => Ok(match img.color.adobe_transform {
            Some(2) => Transform::Ycck,
            _ => Transform::Cmyk,
        }),
        _ => Err(Error::Unsupported(
            "JPEG: only 1, 3, or 4 component streams are supported",
        )),
    }
}

impl JpegDecoder {
    /// The shared marker-loop driver: decodes `data` to an internal [`DecodedImage`].
    fn decode_internal(data: &[u8]) -> Result<DecodedImage> {
        expect_soi(data)?;
        let mut pos = 2;
        let mut tables = Tables::default();
        let mut color = ColorInfo::default();
        let mut frame: Option<Frame> = None;
        let mut planes: Vec<Option<Plane>> = Vec::new();

        loop {
            let (marker, after) = read_marker(data, pos)?;
            match marker {
                code::EOI_CODE => break,
                code::SOF0 | code::SOF1 => {
                    if frame.is_some() {
                        return Err(Error::InvalidInput("JPEG: duplicate frame header"));
                    }
                    let (payload, next) = read_segment(data, after)?;
                    let f = parse_sof(payload)?;
                    planes = (0..f.components.len()).map(|_| None).collect();
                    frame = Some(f);
                    pos = next;
                }
                code::SOF2 => {
                    return Err(Error::Unsupported(
                        "JPEG: progressive DCT (SOF2) decode not yet supported",
                    ));
                }
                code::SOF3 => {
                    return Err(Error::Unsupported(
                        "JPEG: lossless process (SOF3) not supported",
                    ));
                }
                0xC5..=0xC7 => {
                    return Err(Error::Unsupported(
                        "JPEG: hierarchical process (SOF5-7) not supported",
                    ));
                }
                0xC9..=0xCB | 0xCD..=0xCF | code::DAC => {
                    return Err(Error::Unsupported("JPEG: arithmetic coding not supported"));
                }
                code::DHT => {
                    let (payload, next) = read_segment(data, after)?;
                    parse_dht(payload, &mut tables)?;
                    pos = next;
                }
                code::DQT => {
                    let (payload, next) = read_segment(data, after)?;
                    parse_dqt(payload, &mut tables)?;
                    pos = next;
                }
                code::DRI => {
                    let (payload, next) = read_segment(data, after)?;
                    tables.restart_interval = parse_dri(payload)?;
                    pos = next;
                }
                code::SOS => {
                    let f = frame
                        .as_ref()
                        .ok_or(Error::InvalidInput("JPEG: SOS before SOF"))?;
                    let (payload, next) = read_segment(data, after)?;
                    let scan = parse_sos(payload, f)?;
                    for sc in &scan.components {
                        if planes[sc.frame_index].is_some() {
                            return Err(Error::InvalidInput(
                                "JPEG: component coded by more than one scan",
                            ));
                        }
                    }
                    let result = decode_scan(data, next, f, &scan, &tables)?;
                    for (ci, plane) in result.planes {
                        planes[ci] = Some(plane);
                    }
                    pos = result.marker_offset;
                }
                code::DNL => {
                    let (payload, next) = read_segment(data, after)?;
                    let nl = parse_dnl(payload)?;
                    // §B.2.5: DNL defines the height only when Y was 0.
                    if let Some(f) = frame.as_mut()
                        && f.y == 0
                    {
                        f.y = nl;
                    }
                    pos = next;
                }
                code::APP0 => {
                    let (payload, next) = read_segment(data, after)?;
                    parse_app0(payload, &mut color);
                    pos = next;
                }
                code::APP14 => {
                    let (payload, next) = read_segment(data, after)?;
                    parse_app14(payload, &mut color);
                    pos = next;
                }
                code::SOI | code::TEM | code::RST0..=code::RST7 => {
                    return Err(Error::InvalidInput(
                        "JPEG: unexpected standalone marker outside a scan",
                    ));
                }
                _ => {
                    // Any other segment-bearing marker (other APPn, COM, reserved): skip by length.
                    let (_, next) = read_segment(data, after)?;
                    pos = next;
                }
            }
        }

        assemble(frame, planes, color)
    }
}

/// Finalizes a decode: validates the frame is complete, crops each component to its valid region,
/// and packages the [`DecodedImage`].
fn assemble(
    frame: Option<Frame>,
    planes: Vec<Option<Plane>>,
    color: ColorInfo,
) -> Result<DecodedImage> {
    let frame = frame.ok_or(Error::InvalidInput("JPEG: no frame header"))?;
    if frame.y == 0 {
        return Err(Error::InvalidInput(
            "JPEG: frame height Y is 0 and no DNL supplied it",
        ));
    }
    let hmax = usize::from(frame.hmax());
    let vmax = usize::from(frame.vmax());
    let x = usize::from(frame.x);
    let y = usize::from(frame.y);

    let mut comps = Vec::with_capacity(frame.components.len());
    let mut ids = Vec::with_capacity(frame.components.len());
    for (fc, plane) in frame.components.iter().zip(planes) {
        let plane = plane.ok_or(Error::InvalidInput("JPEG: a component was never coded"))?;
        let comp_w = (x * usize::from(fc.h)).div_ceil(hmax);
        let comp_h = (y * usize::from(fc.v)).div_ceil(vmax);
        // The decoded plane must cover the valid region (a DNL claiming more lines than were coded
        // would fail here).
        let padded_rows = plane.data.len().checked_div(plane.stride).unwrap_or(0);
        if comp_w > plane.stride || comp_h > padded_rows {
            return Err(Error::InvalidInput(
                "JPEG: decoded plane smaller than the frame declares",
            ));
        }
        comps.push(DecComp {
            h: plane.h,
            v: plane.v,
            comp_w,
            comp_h,
            stride: plane.stride,
            data: plane.data,
        });
        ids.push(fc.id);
    }

    Ok(DecodedImage {
        width: frame.x.into(),
        height: frame.y.into(),
        hmax,
        vmax,
        comps,
        ids,
        color,
    })
}

/// Presents a decoded image as interleaved 8-bit RGB.
fn present_rgb(img: &DecodedImage) -> Result<Vec<u8>> {
    let (w, h) = (img.width as usize, img.height as usize);
    let transform = decide_transform(img)?;
    let mut out = Vec::with_capacity(w * h * 3);
    for py in 0..h {
        for px in 0..w {
            match transform {
                Transform::Gray => {
                    let g = img.sample(0, px, py);
                    out.extend_from_slice(&[g, g, g]);
                }
                Transform::Rgb => {
                    out.push(img.sample(0, px, py));
                    out.push(img.sample(1, px, py));
                    out.push(img.sample(2, px, py));
                }
                Transform::YCbCr => {
                    let (r, g, b) = ycbcr_to_rgb(
                        img.sample(0, px, py),
                        img.sample(1, px, py),
                        img.sample(2, px, py),
                        ColorRange::Full,
                    );
                    out.extend_from_slice(&[r, g, b]);
                }
                Transform::Cmyk | Transform::Ycck => {
                    return Err(Error::Unsupported(
                        "JPEG: 4-component (CMYK/YCCK) — decode as Cmyk8",
                    ));
                }
            }
        }
    }
    Ok(out)
}

/// Presents a decoded image as interleaved 8-bit grayscale (single-component streams only).
fn present_gray(img: &DecodedImage) -> Result<Vec<u8>> {
    if decide_transform(img)? != Transform::Gray {
        return Err(Error::Unsupported(
            "JPEG: not a single-component grayscale image",
        ));
    }
    let (w, h) = (img.width as usize, img.height as usize);
    let mut out = Vec::with_capacity(w * h);
    for py in 0..h {
        for px in 0..w {
            out.push(img.sample(0, px, py));
        }
    }
    Ok(out)
}

/// Presents a decoded image as interleaved 8-bit CMYK (four-component CMYK/YCCK streams only).
fn present_cmyk(img: &DecodedImage) -> Result<Vec<u8>> {
    let transform = decide_transform(img)?;
    let (w, h) = (img.width as usize, img.height as usize);
    let mut out = Vec::with_capacity(w * h * 4);
    for py in 0..h {
        for px in 0..w {
            match transform {
                Transform::Cmyk => {
                    out.push(img.sample(0, px, py));
                    out.push(img.sample(1, px, py));
                    out.push(img.sample(2, px, py));
                    out.push(img.sample(3, px, py));
                }
                Transform::Ycck => {
                    // YCCK → CMYK: invert the YCbCr transform on the first three channels, K passes
                    // through (Adobe TN #5116).
                    let (r, g, b) = ycbcr_to_rgb(
                        img.sample(0, px, py),
                        img.sample(1, px, py),
                        img.sample(2, px, py),
                        ColorRange::Full,
                    );
                    out.push(255 - r);
                    out.push(255 - g);
                    out.push(255 - b);
                    out.push(img.sample(3, px, py));
                }
                _ => {
                    return Err(Error::Unsupported(
                        "JPEG: not a 4-component CMYK/YCCK image",
                    ));
                }
            }
        }
    }
    Ok(out)
}

impl DecodeImage<Rgb8> for JpegDecoder {
    /// Grayscale is replicated across channels; three-component YCbCr/RGB is presented as RGB;
    /// four-component (CMYK/YCCK) returns [`Error::Unsupported`] (decode as [`Cmyk8`]).
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<Rgb8>> {
        let img = Self::decode_internal(data)?;
        let dims = Dimensions::new(img.width, img.height)?;
        ImageBuf::new(present_rgb(&img)?, dims)
    }
}

impl DecodeImage<Gray8> for JpegDecoder {
    /// Errors unless the stream is single-component; the luminance samples pass through unchanged.
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<Gray8>> {
        let img = Self::decode_internal(data)?;
        let dims = Dimensions::new(img.width, img.height)?;
        ImageBuf::new(present_gray(&img)?, dims)
    }
}

impl DecodeImage<Cmyk8> for JpegDecoder {
    /// Errors unless the stream is four-component; CMYK passes through, YCCK is inverted to CMYK.
    fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<Cmyk8>> {
        let img = Self::decode_internal(data)?;
        let dims = Dimensions::new(img.width, img.height)?;
        ImageBuf::new(present_cmyk(&img)?, dims)
    }
}

#[cfg(test)]
mod tests {
    //! Hand-built stream tests: the decoder is driven by JPEG byte vectors assembled from the
    //! crate's own marker/table writers plus a private entropy emitter (the inverse of §F.2), so
    //! each colour path, scan layout, and marker case is exercised against an independently computed
    //! expectation. These cover ground the encoder round-trips cannot (non-interleaved multi-scan,
    //! CMYK/YCCK, Adobe/RGB colour, SOF1).

    use gamut_color::{ColorRange, ycbcr_to_rgb};
    use gamut_core::{Cmyk8, DecodeImage, Error, Gray8, ImageBuf, Rgb8};
    use gamut_dsp::jpeg::idct8x8;

    use super::*;
    use crate::bitwriter::BitWriter;
    use crate::huffman::{self, EncTable};
    use crate::zigzag::ZIGZAG;
    use crate::{marker, quant};

    /// Magnitude category (bit length of `|value|`; 0 for 0) — the §F.1.2 SSSS.
    fn magcat(value: i32) -> u8 {
        (32 - value.unsigned_abs().leading_zeros()) as u8
    }

    /// The SSSS additional bits for `value` (the encoder's inverse of `EXTEND`).
    fn addbits(value: i32, cat: u8) -> u16 {
        let v = if value < 0 { value - 1 } else { value };
        (v as u32 & ((1u32 << cat) - 1)) as u16
    }

    /// Emits one block's DC + AC entropy for natural-order quantized `coeffs` against the running
    /// `pred` (§F.1.2), mirroring the encoder's coder so the decoder can be pinned independently.
    fn emit_block(
        w: &mut BitWriter,
        coeffs: &[i32; 64],
        pred: &mut i32,
        dc: &EncTable,
        ac: &EncTable,
    ) {
        let diff = coeffs[0] - *pred;
        *pred = coeffs[0];
        let cat = magcat(diff);
        let (c, l) = dc.lookup(cat).unwrap();
        w.write_bits(c, l);
        w.write_bits(addbits(diff, cat), cat);
        let mut run = 0u8;
        for &nat in &ZIGZAG[1..] {
            let v = coeffs[nat];
            if v == 0 {
                run += 1;
                continue;
            }
            while run >= 16 {
                let (c, l) = ac.lookup(0xF0).unwrap();
                w.write_bits(c, l);
                run -= 16;
            }
            let cat = magcat(v);
            let (c, l) = ac.lookup((run << 4) | cat).unwrap();
            w.write_bits(c, l);
            w.write_bits(addbits(v, cat), cat);
            run = 0;
        }
        if run > 0 {
            let (c, l) = ac.lookup(0x00).unwrap();
            w.write_bits(c, l);
        }
    }

    /// Encodes an ordered list of `(component_table_index, block)` into a flushed entropy segment,
    /// resetting one predictor per table slot.
    fn entropy(order: &[(usize, [i32; 64])], tables: &[(EncTable, EncTable)]) -> Vec<u8> {
        let mut preds = vec![0i32; tables.len()];
        let mut out = Vec::new();
        let mut w = BitWriter::new(&mut out);
        for (ci, coeffs) in order {
            emit_block(
                &mut w,
                coeffs,
                &mut preds[*ci],
                &tables[*ci].0,
                &tables[*ci].1,
            );
        }
        w.flush();
        out
    }

    /// The four standard `(EncTable)` pairs used throughout: `(luma_dc, luma_ac, chroma_dc,
    /// chroma_ac)`.
    fn std_enc() -> (EncTable, EncTable, EncTable, EncTable) {
        (
            EncTable::from_spec(&huffman::STD_LUMA_DC),
            EncTable::from_spec(&huffman::STD_LUMA_AC),
            EncTable::from_spec(&huffman::STD_CHROMA_DC),
            EncTable::from_spec(&huffman::STD_CHROMA_AC),
        )
    }

    /// Appends an Adobe APP14 segment carrying `transform` (TN #5116).
    fn write_app14(out: &mut Vec<u8>, transform: u8) {
        marker::write_segment_header(out, marker::code::APP14, 14);
        out.extend_from_slice(b"Adobe");
        out.extend_from_slice(&[0x00, 0x64]); // version 100
        out.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]); // flags0, flags1
        out.push(transform);
    }

    /// A natural-order block with only the DC coefficient set — a flat block.
    fn flat(dc: i32) -> [i32; 64] {
        let mut b = [0i32; 64];
        b[0] = dc;
        b
    }

    /// A quantization table whose DC step is 8 (so a flat block of quantized DC `d` reconstructs to
    /// the uniform sample `128 + d`) and all other steps 1.
    fn dc8_quant() -> [u8; 64] {
        let mut q = [1u8; 64];
        q[0] = 8;
        q
    }

    #[test]
    fn exact_8x8_gray_block() {
        // A single 8×8 gray block with hand-chosen quantized coefficients decoded to exact pixels,
        // computed independently: dequantize (coeff · Q, natural order) → idct8x8 → +128 → clamp.
        let quant = quant::LUMINANCE; // a non-trivial Annex K table exercises the AC dequant multiply
        let mut coeffs = [0i32; 64];
        coeffs[0] = 5; // DC
        coeffs[1] = -3; // first AC (natural index 1 = zig-zag position 1)
        coeffs[ZIGZAG[5]] = 2; // an AC further along the zig-zag
        coeffs[ZIGZAG[16]] = 1;

        // Independent expected pixels.
        let mut zz = [0i32; 64];
        for (i, cell) in zz.iter_mut().enumerate() {
            *cell = coeffs[i] * i32::from(quant[i]);
        }
        idct8x8(&mut zz);
        let expected: Vec<u8> = zz.iter().map(|&s| (s + 128).clamp(0, 255) as u8).collect();

        let (ldc, lac, ..) = std_enc();
        let body = entropy(&[(0, coeffs)], &[(ldc, lac)]);

        let mut jpeg = Vec::new();
        marker::write_marker(&mut jpeg, marker::code::SOI);
        quant::emit_dqt(&mut jpeg, &[(0, &quant)]);
        marker::write_sof0(&mut jpeg, 8, 8, &[(1, 1, 1, 0)]);
        huffman::emit_dht(
            &mut jpeg,
            &[(0, 0, &huffman::STD_LUMA_DC), (1, 0, &huffman::STD_LUMA_AC)],
        );
        marker::write_sos(&mut jpeg, &[(1, 0, 0)]);
        jpeg.extend_from_slice(&body);
        marker::write_marker(&mut jpeg, marker::code::EOI);

        let out: ImageBuf<Gray8> = JpegDecoder::new().decode_image(&jpeg).unwrap();
        assert_eq!(out.as_samples(), expected.as_slice());
    }

    /// Builds a 16×16 three-component 4:4:4 YCbCr stream. `scans` lists, per SOS, the `(cs, td, ta)`
    /// component triples; the entropy for each scan is built from the shared `grids` (per component,
    /// a 2×2 block grid in row-major order). This lets the interleaved and non-interleaved layouts
    /// be produced from identical coefficients.
    fn ycbcr444_16(grids: &[[[i32; 64]; 4]; 3], interleaved: bool) -> Vec<u8> {
        let (ldc, lac, cdc, cac) = std_enc();
        let mut jpeg = Vec::new();
        marker::write_marker(&mut jpeg, marker::code::SOI);
        marker::write_app0_jfif(&mut jpeg, marker::DensityUnit::AspectRatio, 1, 1);
        quant::emit_dqt(&mut jpeg, &[(0, &[1u8; 64]), (1, &[1u8; 64])]);
        marker::write_sof0(
            &mut jpeg,
            16,
            16,
            &[(1, 1, 1, 0), (2, 1, 1, 1), (3, 1, 1, 1)],
        );
        huffman::emit_dht(
            &mut jpeg,
            &[
                (0, 0, &huffman::STD_LUMA_DC),
                (1, 0, &huffman::STD_LUMA_AC),
                (0, 1, &huffman::STD_CHROMA_DC),
                (1, 1, &huffman::STD_CHROMA_AC),
            ],
        );
        // Component table pairs by frame index: Y uses luma tables, Cb/Cr chroma tables.
        let comp_tables = |ci: usize| {
            if ci == 0 {
                (ldc.clone(), lac.clone())
            } else {
                (cdc.clone(), cac.clone())
            }
        };
        if interleaved {
            marker::write_sos(&mut jpeg, &[(1, 0, 0), (2, 1, 1), (3, 1, 1)]);
            // MCU order: for each of the 4 MCUs (row-major), one block per component.
            let mut order = Vec::new();
            for mcu in 0..4 {
                for (ci, grid) in grids.iter().enumerate() {
                    order.push((ci, grid[mcu]));
                }
            }
            let tables = [comp_tables(0), comp_tables(1), comp_tables(2)];
            jpeg.extend_from_slice(&entropy(&order, &tables));
        } else {
            for (ci, grid) in grids.iter().enumerate() {
                let cs = (ci + 1) as u8;
                let (td, ta) = if ci == 0 { (0, 0) } else { (1, 1) };
                marker::write_sos(&mut jpeg, &[(cs, td, ta)]);
                let order: Vec<(usize, [i32; 64])> =
                    grid.iter().map(|blk| (0usize, *blk)).collect();
                jpeg.extend_from_slice(&entropy(&order, &[comp_tables(ci)]));
            }
        }
        marker::write_marker(&mut jpeg, marker::code::EOI);
        jpeg
    }

    /// Builds a 16×16 4:2:0 stream (Y at 2×2, Cb/Cr at 1×1) from a 4-block luma grid (row-major) and
    /// one Cb/one Cr block, as either one interleaved scan or three non-interleaved scans. This
    /// exercises the component-dimension arithmetic (§A.1.1) for a component with `Hi = Vi = 2`.
    fn ycbcr420_16(
        luma: [[i32; 64]; 4],
        cb: [i32; 64],
        cr: [i32; 64],
        interleaved: bool,
    ) -> Vec<u8> {
        let (ldc, lac, cdc, cac) = std_enc();
        let mut jpeg = Vec::new();
        marker::write_marker(&mut jpeg, marker::code::SOI);
        marker::write_app0_jfif(&mut jpeg, marker::DensityUnit::AspectRatio, 1, 1);
        quant::emit_dqt(&mut jpeg, &[(0, &[1u8; 64]), (1, &[1u8; 64])]);
        marker::write_sof0(
            &mut jpeg,
            16,
            16,
            &[(1, 2, 2, 0), (2, 1, 1, 1), (3, 1, 1, 1)],
        );
        huffman::emit_dht(
            &mut jpeg,
            &[
                (0, 0, &huffman::STD_LUMA_DC),
                (1, 0, &huffman::STD_LUMA_AC),
                (0, 1, &huffman::STD_CHROMA_DC),
                (1, 1, &huffman::STD_CHROMA_AC),
            ],
        );
        if interleaved {
            marker::write_sos(&mut jpeg, &[(1, 0, 0), (2, 1, 1), (3, 1, 1)]);
            // One MCU: the four luma blocks (row-major) then Cb then Cr.
            let order = vec![
                (0usize, luma[0]),
                (0, luma[1]),
                (0, luma[2]),
                (0, luma[3]),
                (1, cb),
                (2, cr),
            ];
            let tables = [
                (ldc.clone(), lac.clone()),
                (cdc.clone(), cac.clone()),
                (cdc, cac),
            ];
            jpeg.extend_from_slice(&entropy(&order, &tables));
        } else {
            marker::write_sos(&mut jpeg, &[(1, 0, 0)]);
            let luma_order: Vec<(usize, [i32; 64])> = luma.iter().map(|b| (0usize, *b)).collect();
            jpeg.extend_from_slice(&entropy(&luma_order, &[(ldc, lac.clone())]));
            marker::write_sos(&mut jpeg, &[(2, 1, 1)]);
            jpeg.extend_from_slice(&entropy(&[(0, cb)], &[(cdc.clone(), cac.clone())]));
            marker::write_sos(&mut jpeg, &[(3, 1, 1)]);
            jpeg.extend_from_slice(&entropy(&[(0, cr)], &[(cdc, cac)]));
        }
        marker::write_marker(&mut jpeg, marker::code::EOI);
        jpeg
    }

    #[test]
    fn non_interleaved_420_matches_interleaved() {
        // A subsampled (4:2:0) frame coded as one interleaved scan vs three non-interleaved scans
        // must decode identically — pinning the component-dimension arithmetic for Hi = Vi = 2.
        let mut luma = [[0i32; 64]; 4];
        for (i, blk) in luma.iter_mut().enumerate() {
            blk[0] = i as i32 * 4 - 6;
            blk[1] = i as i32 - 2;
        }
        let mut cb = [0i32; 64];
        cb[0] = 5;
        let mut cr = [0i32; 64];
        cr[0] = -7;
        let inter = ycbcr420_16(luma, cb, cr, true);
        let non = ycbcr420_16(luma, cb, cr, false);
        let a: ImageBuf<Rgb8> = JpegDecoder::new().decode_image(&inter).unwrap();
        let b: ImageBuf<Rgb8> = JpegDecoder::new().decode_image(&non).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.dimensions(), gamut_core::Dimensions::new(16, 16).unwrap());
    }

    #[test]
    fn non_interleaved_three_scans_match_interleaved() {
        // Identical coefficients emitted as one interleaved scan vs three non-interleaved scans must
        // decode to the exact same image — pinning both the MCU walk and the multi-scan driver.
        let mut grids = [[[0i32; 64]; 4]; 3];
        for (ci, grid) in grids.iter_mut().enumerate() {
            for (b, blk) in grid.iter_mut().enumerate() {
                // Distinct DC per (component, block) plus a low AC, so block placement is testable.
                blk[0] = (ci as i32 * 7 + b as i32 * 3) - 6;
                blk[1] = ci as i32 - 1;
            }
        }
        let inter = ycbcr444_16(&grids, true);
        let non = ycbcr444_16(&grids, false);
        let a: ImageBuf<Rgb8> = JpegDecoder::new().decode_image(&inter).unwrap();
        let b: ImageBuf<Rgb8> = JpegDecoder::new().decode_image(&non).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn sof1_decodes_like_sof0() {
        // Flipping the SOF0 marker (0xC0) to SOF1 (0xC1) must not change the decoded image.
        let mut g = [[[0i32; 64]; 4]; 3];
        for (ci, grid) in g.iter_mut().enumerate() {
            for (b, blk) in grid.iter_mut().enumerate() {
                blk[0] = ci as i32 * 5 + b as i32;
            }
        }
        let base = ycbcr444_16(&g, true);
        let mut sof1 = base.clone();
        let idx = sof1.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
        sof1[idx + 1] = 0xC1;
        let a: ImageBuf<Rgb8> = JpegDecoder::new().decode_image(&base).unwrap();
        let b: ImageBuf<Rgb8> = JpegDecoder::new().decode_image(&sof1).unwrap();
        assert_eq!(a, b);
    }

    /// Builds an 8×8 four-component stream (ids 1..4, all 1×1, quant [`dc8_quant`]) with flat blocks
    /// of quantized DC `d`, optionally tagged with an Adobe APP14 `transform`.
    fn four_comp_8x8(d: [i32; 4], app14: Option<u8>) -> Vec<u8> {
        let (ldc, lac, ..) = std_enc();
        let quant = dc8_quant();
        let mut jpeg = Vec::new();
        marker::write_marker(&mut jpeg, marker::code::SOI);
        if let Some(t) = app14 {
            write_app14(&mut jpeg, t);
        }
        quant::emit_dqt(&mut jpeg, &[(0, &quant)]);
        marker::write_sof0(
            &mut jpeg,
            8,
            8,
            &[(1, 1, 1, 0), (2, 1, 1, 0), (3, 1, 1, 0), (4, 1, 1, 0)],
        );
        huffman::emit_dht(
            &mut jpeg,
            &[(0, 0, &huffman::STD_LUMA_DC), (1, 0, &huffman::STD_LUMA_AC)],
        );
        marker::write_sos(&mut jpeg, &[(1, 0, 0), (2, 0, 0), (3, 0, 0), (4, 0, 0)]);
        let tables: Vec<(EncTable, EncTable)> =
            (0..4).map(|_| (ldc.clone(), lac.clone())).collect();
        let order: Vec<(usize, [i32; 64])> = (0..4).map(|ci| (ci, flat(d[ci]))).collect();
        jpeg.extend_from_slice(&entropy(&order, &tables));
        marker::write_marker(&mut jpeg, marker::code::EOI);
        jpeg
    }

    #[test]
    fn cmyk_four_component_passthrough() {
        // No APP14 → CMYK stored verbatim. Flat DCs 10/20/30/40 → uniform 138/148/158/168.
        let jpeg = four_comp_8x8([10, 20, 30, 40], None);
        let out: ImageBuf<Cmyk8> = JpegDecoder::new().decode_image(&jpeg).unwrap();
        assert_eq!(&out.as_samples()[..4], &[138, 148, 158, 168]);
        assert!(
            out.as_samples()
                .chunks_exact(4)
                .all(|p| p == [138, 148, 158, 168])
        );
    }

    #[test]
    fn ycck_four_component_inverts_to_cmyk() {
        // APP14 transform=2 → YCCK: the first three channels are inverse-YCbCr'd to RGB, then CMY =
        // 255 − RGB, K passes through.
        let d = [10, -20, 30, 40];
        let jpeg = four_comp_8x8(d, Some(2));
        let (y, cb, cr) = ((128 + d[0]) as u8, (128 + d[1]) as u8, (128 + d[2]) as u8);
        let (r, g, b) = ycbcr_to_rgb(y, cb, cr, ColorRange::Full);
        let expected = [255 - r, 255 - g, 255 - b, (128 + d[3]) as u8];
        let out: ImageBuf<Cmyk8> = JpegDecoder::new().decode_image(&jpeg).unwrap();
        assert_eq!(&out.as_samples()[..4], &expected);
    }

    /// Builds an 8×8 three-component stream (given component ids, all 1×1, quant [`dc8_quant`]) with
    /// flat blocks, optionally tagged with a JFIF APP0 or Adobe APP14.
    fn three_comp_8x8(ids: [u8; 3], d: [i32; 3], jfif: bool, app14: Option<u8>) -> Vec<u8> {
        let (ldc, lac, ..) = std_enc();
        let quant = dc8_quant();
        let mut jpeg = Vec::new();
        marker::write_marker(&mut jpeg, marker::code::SOI);
        if jfif {
            marker::write_app0_jfif(&mut jpeg, marker::DensityUnit::AspectRatio, 1, 1);
        }
        if let Some(t) = app14 {
            write_app14(&mut jpeg, t);
        }
        quant::emit_dqt(&mut jpeg, &[(0, &quant)]);
        marker::write_sof0(
            &mut jpeg,
            8,
            8,
            &[(ids[0], 1, 1, 0), (ids[1], 1, 1, 0), (ids[2], 1, 1, 0)],
        );
        huffman::emit_dht(
            &mut jpeg,
            &[(0, 0, &huffman::STD_LUMA_DC), (1, 0, &huffman::STD_LUMA_AC)],
        );
        marker::write_sos(&mut jpeg, &[(ids[0], 0, 0), (ids[1], 0, 0), (ids[2], 0, 0)]);
        let tables: Vec<(EncTable, EncTable)> =
            (0..3).map(|_| (ldc.clone(), lac.clone())).collect();
        let order: Vec<(usize, [i32; 64])> = (0..3).map(|ci| (ci, flat(d[ci]))).collect();
        jpeg.extend_from_slice(&entropy(&order, &tables));
        marker::write_marker(&mut jpeg, marker::code::EOI);
        jpeg
    }

    #[test]
    fn adobe_transform0_three_component_is_rgb_passthrough() {
        // APP14 transform=0 with 3 components → the samples are R,G,B directly (no colour transform).
        let d = [10, 20, 30];
        let jpeg = three_comp_8x8([1, 2, 3], d, false, Some(0));
        let out: ImageBuf<Rgb8> = JpegDecoder::new().decode_image(&jpeg).unwrap();
        assert_eq!(&out.as_samples()[..3], &[138, 148, 158]);
    }

    #[test]
    fn component_ids_rgb_without_app14_is_rgb() {
        // Component ids 'R','G','B' and no JFIF/Adobe → RGB passthrough (the de-facto rule).
        let jpeg = three_comp_8x8(*b"RGB", [10, 20, 30], false, None);
        let out: ImageBuf<Rgb8> = JpegDecoder::new().decode_image(&jpeg).unwrap();
        assert_eq!(&out.as_samples()[..3], &[138, 148, 158]);
    }

    #[test]
    fn jfif_three_component_is_ycbcr() {
        // A JFIF APP0 (and ids 1,2,3, no Adobe) selects the YCbCr transform.
        let d = [10, -20, 30];
        let jpeg = three_comp_8x8([1, 2, 3], d, true, None);
        let (r, g, b) = ycbcr_to_rgb(
            (128 + d[0]) as u8,
            (128 + d[1]) as u8,
            (128 + d[2]) as u8,
            ColorRange::Full,
        );
        let out: ImageBuf<Rgb8> = JpegDecoder::new().decode_image(&jpeg).unwrap();
        assert_eq!(&out.as_samples()[..3], &[r, g, b]);
    }

    #[test]
    fn jfif_overrides_rgb_component_ids() {
        // Component ids 'R','G','B' would default to RGB, but a JFIF APP0 forces YCbCr — the decision
        // tree checks JFIF before the id heuristic. This pins that the APP0 flag is actually read.
        let d = [10, -20, 30];
        let jpeg = three_comp_8x8(*b"RGB", d, true, None);
        let (r, g, b) = ycbcr_to_rgb(
            (128 + d[0]) as u8,
            (128 + d[1]) as u8,
            (128 + d[2]) as u8,
            ColorRange::Full,
        );
        let out: ImageBuf<Rgb8> = JpegDecoder::new().decode_image(&jpeg).unwrap();
        // Not the RGB passthrough [138, 108, 158]; the YCbCr conversion instead.
        assert_eq!(&out.as_samples()[..3], &[r, g, b]);
    }

    #[test]
    fn undefined_table_and_bad_component_are_rejected() {
        // A SOS whose component references an undefined quant table (Tq=2 never DQT'd) is rejected.
        let (ldc, lac, ..) = std_enc();
        let mut jpeg = Vec::new();
        marker::write_marker(&mut jpeg, marker::code::SOI);
        quant::emit_dqt(&mut jpeg, &[(0, &dc8_quant())]);
        marker::write_sof0(&mut jpeg, 8, 8, &[(1, 1, 1, 2)]); // Tq=2, never defined
        huffman::emit_dht(
            &mut jpeg,
            &[(0, 0, &huffman::STD_LUMA_DC), (1, 0, &huffman::STD_LUMA_AC)],
        );
        marker::write_sos(&mut jpeg, &[(1, 0, 0)]);
        jpeg.extend_from_slice(&entropy(&[(0, flat(1))], &[(ldc, lac)]));
        marker::write_marker(&mut jpeg, marker::code::EOI);
        assert!(
            <JpegDecoder as DecodeImage<Gray8>>::decode_image(&JpegDecoder::new(), &jpeg).is_err()
        );
    }

    #[test]
    fn ac_index_past_63_is_rejected() {
        // A block whose AC run overruns coefficient 63 is malformed: three ZRLs advance the index to
        // 49, then a run/size symbol 0xF1 (run 15, size 1) pushes it to 64 > 63 — a hard reject.
        let (ldc, lac, ..) = std_enc();
        let mut body = Vec::new();
        let mut w = BitWriter::new(&mut body);
        // DC category 0.
        let (c, l) = ldc.lookup(0).unwrap();
        w.write_bits(c, l);
        // Three ZRLs → coefficient index 1 + 3·16 = 49.
        for _ in 0..3 {
            let (c, l) = lac.lookup(0xF0).unwrap();
            w.write_bits(c, l);
        }
        // Symbol 0xF1: run 15 pushes the index to 49 + 15 = 64, past the last coefficient.
        let (c, l) = lac.lookup(0xF1).unwrap();
        w.write_bits(c, l);
        w.write_bits(1, 1);
        w.flush();

        let mut jpeg = Vec::new();
        marker::write_marker(&mut jpeg, marker::code::SOI);
        quant::emit_dqt(&mut jpeg, &[(0, &dc8_quant())]);
        marker::write_sof0(&mut jpeg, 8, 8, &[(1, 1, 1, 0)]);
        huffman::emit_dht(
            &mut jpeg,
            &[(0, 0, &huffman::STD_LUMA_DC), (1, 0, &huffman::STD_LUMA_AC)],
        );
        marker::write_sos(&mut jpeg, &[(1, 0, 0)]);
        jpeg.extend_from_slice(&body);
        marker::write_marker(&mut jpeg, marker::code::EOI);
        assert!(
            <JpegDecoder as DecodeImage<Gray8>>::decode_image(&JpegDecoder::new(), &jpeg).is_err()
        );
    }

    /// Builds a single-component (gray) stream `x` wide with `y_field` in the SOF `Y` field, coding
    /// the flat blocks `blocks` (one block column, top to bottom), optionally followed by a raw DNL
    /// segment (`dnl_len`/`dnl_nl`) and always closed by EOI.
    fn gray_stream(x: u16, y_field: u16, blocks: &[[i32; 64]], dnl: Option<(u16, u16)>) -> Vec<u8> {
        let (ldc, lac, ..) = std_enc();
        let mut jpeg = Vec::new();
        marker::write_marker(&mut jpeg, marker::code::SOI);
        quant::emit_dqt(&mut jpeg, &[(0, &dc8_quant())]);
        marker::write_sof0(&mut jpeg, x, y_field, &[(1, 1, 1, 0)]);
        huffman::emit_dht(
            &mut jpeg,
            &[(0, 0, &huffman::STD_LUMA_DC), (1, 0, &huffman::STD_LUMA_AC)],
        );
        marker::write_sos(&mut jpeg, &[(1, 0, 0)]);
        let order: Vec<(usize, [i32; 64])> = blocks.iter().map(|b| (0usize, *b)).collect();
        jpeg.extend_from_slice(&entropy(&order, &[(ldc, lac)]));
        if let Some((len, nl)) = dnl {
            marker::write_segment_header(&mut jpeg, marker::code::DNL, usize::from(len));
            jpeg.extend_from_slice(&nl.to_be_bytes());
        }
        marker::write_marker(&mut jpeg, marker::code::EOI);
        jpeg
    }

    #[test]
    fn zero_height_resolved_by_dnl() {
        // Y=0 in the SOF, two flat block rows decoded until the entropy ends, then DNL(16) supplies
        // the height. Exercises the unknown-row MCU loop, the marker-boundary end detection, and the
        // DNL height application (§B.2.5). A fill 0xFF is inserted before the DNL marker so the
        // end-of-data peek must skip it (§B.1.1.2).
        let mut jpeg = gray_stream(8, 0, &[flat(10), flat(-30)], Some((4, 16)));
        let dnl = jpeg.windows(2).position(|w| w == [0xFF, 0xDC]).unwrap();
        jpeg.insert(dnl, 0xFF); // fill byte: now 0xFF 0xFF 0xDC
        let out: ImageBuf<Gray8> = JpegDecoder::new().decode_image(&jpeg).unwrap();
        assert_eq!(
            out.dimensions(),
            gamut_core::Dimensions::new(8, 16).unwrap()
        );
        assert!(out.as_samples()[..64].iter().all(|&s| s == 138)); // top block: 128 + 10
        assert!(out.as_samples()[64..].iter().all(|&s| s == 98)); // bottom block: 128 − 30
    }

    #[test]
    fn zero_height_without_dnl_is_rejected() {
        // Y=0 and the scan ends at EOI with no DNL: the height is never defined → InvalidInput.
        let jpeg = gray_stream(8, 0, &[flat(5)], None);
        assert!(
            <JpegDecoder as DecodeImage<Gray8>>::decode_image(&JpegDecoder::new(), &jpeg).is_err()
        );
    }

    #[test]
    fn dnl_is_ignored_when_height_is_known() {
        // A DNL after a normal Y≠0 frame is advisory and ignored (§B.2.5): the image still decodes at
        // the SOF-declared height.
        let jpeg = gray_stream(8, 8, &[flat(20)], Some((4, 999)));
        let out: ImageBuf<Gray8> = JpegDecoder::new().decode_image(&jpeg).unwrap();
        assert_eq!(out.dimensions(), gamut_core::Dimensions::new(8, 8).unwrap());
        // A malformed DNL (NL=0) is rejected even though the height is already known.
        let bad = gray_stream(8, 8, &[flat(20)], Some((4, 0)));
        assert!(
            <JpegDecoder as DecodeImage<Gray8>>::decode_image(&JpegDecoder::new(), &bad).is_err()
        );
        // A DNL with the wrong segment length is rejected.
        let bad_len = gray_stream(8, 8, &[flat(20)], Some((5, 8)));
        assert!(
            <JpegDecoder as DecodeImage<Gray8>>::decode_image(&JpegDecoder::new(), &bad_len)
                .is_err()
        );
    }

    #[test]
    fn fill_bytes_before_marker_are_skipped() {
        // A fill 0xFF before the terminating EOI (§B.1.1.2) must be skipped by the entropy reader.
        let mut jpeg = gray_stream(8, 8, &[flat(7)], None);
        let eoi = jpeg.len() - 2; // the 0xFF of the final EOI
        jpeg.insert(eoi, 0xFF); // now 0xFF 0xFF 0xD9
        let out: ImageBuf<Gray8> = JpegDecoder::new().decode_image(&jpeg).unwrap();
        assert!(out.as_samples().iter().all(|&s| s == 135)); // 128 + 7
    }

    #[test]
    fn four_component_cannot_present_as_rgb() {
        // The Rgb8 impl rejects a 4-component (CMYK) stream — the caller must use Cmyk8.
        let jpeg = four_comp_8x8([10, 20, 30, 40], None);
        assert!(
            <JpegDecoder as DecodeImage<Rgb8>>::decode_image(&JpegDecoder::new(), &jpeg).is_err()
        );
    }

    #[test]
    fn unsupported_and_structural_markers_are_classified() {
        let base = gray_stream(8, 8, &[flat(3)], None);
        let sof = base.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
        let flip = |code: u8| {
            let mut m = base.clone();
            m[sof + 1] = code;
            m
        };
        // info() distinguishes each supported process by its SOFn marker.
        assert_eq!(info(&base).unwrap().process, JpegProcess::Baseline);
        assert_eq!(
            info(&flip(0xC1)).unwrap().process,
            JpegProcess::ExtendedSequential
        );
        assert_eq!(info(&flip(0xC2)).unwrap().process, JpegProcess::Progressive);
        // The unsupported processes decode to a specific `Unsupported` error (not merely any error).
        for code in [0xC2u8, 0xC3, 0xC5, 0xC9, 0xCC, 0xCF] {
            let m = flip(code);
            assert!(
                matches!(
                    <JpegDecoder as DecodeImage<Gray8>>::decode_image(&JpegDecoder::new(), &m),
                    Err(Error::Unsupported(_))
                ),
                "marker {code:#x} must be Unsupported"
            );
        }
        // info() reports Unsupported for an unsupported process, InvalidInput for a scan-before-frame
        // stream (SOS with no preceding SOF).
        assert!(matches!(info(&flip(0xC3)), Err(Error::Unsupported(_))));
        let early = [0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x08, 1, 1, 0x00, 0, 63, 0];
        assert!(matches!(info(&early), Err(Error::InvalidInput(_))));

        // Duplicate SOF: splice a second SOF0 segment before SOS → InvalidInput.
        let sos = base.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
        let mut dup = base.clone();
        let mut extra = Vec::new();
        marker::write_sof0(&mut extra, 8, 8, &[(1, 1, 1, 0)]);
        dup.splice(sos..sos, extra);
        assert!(matches!(
            <JpegDecoder as DecodeImage<Gray8>>::decode_image(&JpegDecoder::new(), &dup),
            Err(Error::InvalidInput(_))
        ));
    }

    #[test]
    fn corrupt_soi_on_otherwise_valid_stream_is_rejected() {
        // The rest of the stream is valid, so only the SOI check can reject it — pinning that the
        // first two bytes really are validated as 0xFF 0xD8.
        let mut s = gray_stream(8, 8, &[flat(5)], None);
        s[0] = 0x00;
        s[1] = 0x00;
        assert!(
            <JpegDecoder as DecodeImage<Gray8>>::decode_image(&JpegDecoder::new(), &s).is_err()
        );
    }

    #[test]
    fn empty_advisory_segment_is_accepted() {
        // A length-2 (empty payload) COM segment is valid and must not be rejected by the segment
        // length check (`len < 2` boundary).
        let mut s = gray_stream(8, 8, &[flat(9)], None);
        let dqt = s.windows(2).position(|w| w == [0xFF, 0xDB]).unwrap();
        s.splice(dqt..dqt, [0xFF, 0xFE, 0x00, 0x02]); // COM marker, empty payload
        let out: ImageBuf<Gray8> = JpegDecoder::new().decode_image(&s).unwrap();
        assert!(out.as_samples().iter().all(|&v| v == 137)); // 128 + 9
    }

    #[test]
    fn dnl_claiming_more_lines_than_coded_is_rejected() {
        // Y=0, one 8-row block decoded, then DNL claims 100 lines — more than were coded. The
        // plane-coverage check in `assemble` must reject it (a lenient `&&` there would read past the
        // plane).
        let jpeg = gray_stream(8, 0, &[flat(5)], Some((4, 100)));
        assert!(
            <JpegDecoder as DecodeImage<Gray8>>::decode_image(&JpegDecoder::new(), &jpeg).is_err()
        );
    }

    #[test]
    fn dc_category_11_is_accepted() {
        // A DC difference of category 11 (magnitude 1024–2047) is the largest legal 8-bit DC
        // category; the `> 11` guard must admit it. Quantized DC 1024 · step 8 → 8192, IDCT flat →
        // 1024, +128 → clamps to 255.
        let jpeg = gray_stream(8, 8, &[flat(1024)], None);
        let out: ImageBuf<Gray8> = JpegDecoder::new().decode_image(&jpeg).unwrap();
        assert!(out.as_samples().iter().all(|&v| v == 255));
    }

    #[test]
    fn crafted_table_and_scan_segments_are_validated() {
        let d =
            |b: &[u8]| <JpegDecoder as DecodeImage<Gray8>>::decode_image(&JpegDecoder::new(), b);
        // DHT with class Tc=2 (only 0/1 valid).
        let mut dht = Vec::new();
        marker::write_marker(&mut dht, marker::code::SOI);
        quant::emit_dqt(&mut dht, &[(0, &dc8_quant())]);
        marker::write_sof0(&mut dht, 8, 8, &[(1, 1, 1, 0)]);
        // A hand-written DHT segment with Tc=2: length 2 + 1 + 16 + 0 = 19, no values.
        marker::write_segment_header(&mut dht, marker::code::DHT, 19);
        dht.push(0x20); // Tc=2, Th=0
        dht.extend_from_slice(&[0u8; 16]);
        marker::write_sos(&mut dht, &[(1, 0, 0)]);
        marker::write_marker(&mut dht, marker::code::EOI);
        assert!(d(&dht).is_err(), "DHT Tc=2");

        // DRI with a 5-byte (wrong) length.
        let mut dri = Vec::new();
        marker::write_marker(&mut dri, marker::code::SOI);
        marker::write_segment_header(&mut dri, marker::code::DRI, 5);
        dri.extend_from_slice(&[0, 1, 0]);
        marker::write_marker(&mut dri, marker::code::EOI);
        assert!(d(&dri).is_err(), "DRI bad length");

        // A 16-bit DQT (Pq=1) carrying a zero value is rejected.
        let mut dqt = Vec::new();
        marker::write_marker(&mut dqt, marker::code::SOI);
        marker::write_segment_header(&mut dqt, marker::code::DQT, 2 + 1 + 128);
        dqt.push(0x10); // Pq=1, Tq=0
        dqt.extend_from_slice(&[0u8; 128]); // all-zero 16-bit values
        marker::write_marker(&mut dqt, marker::code::EOI);
        assert!(d(&dqt).is_err(), "16-bit DQT zero value");

        // SOS with a duplicate component selector.
        let mut sos = Vec::new();
        marker::write_marker(&mut sos, marker::code::SOI);
        quant::emit_dqt(&mut sos, &[(0, &dc8_quant())]);
        marker::write_sof0(&mut sos, 8, 8, &[(1, 1, 1, 0), (2, 1, 1, 0)]);
        huffman::emit_dht(
            &mut sos,
            &[(0, 0, &huffman::STD_LUMA_DC), (1, 0, &huffman::STD_LUMA_AC)],
        );
        marker::write_sos(&mut sos, &[(1, 0, 0), (1, 0, 0)]); // component 1 twice
        marker::write_marker(&mut sos, marker::code::EOI);
        assert!(d(&sos).is_err(), "duplicate SOS component");

        // SOS appearing before any SOF.
        let early = [0xFF, 0xD8, 0xFF, 0xDA, 0x00, 0x08, 1, 1, 0x00, 0, 63, 0];
        assert!(d(&early).is_err(), "SOS before SOF");

        // A SOS with Ns=0 (empty component list) is rejected — a lenient count check would let it
        // through to a zero-component scan.
        let mut ns0 = Vec::new();
        marker::write_marker(&mut ns0, marker::code::SOI);
        quant::emit_dqt(&mut ns0, &[(0, &dc8_quant())]);
        marker::write_sof0(&mut ns0, 8, 8, &[(1, 1, 1, 0)]);
        huffman::emit_dht(
            &mut ns0,
            &[(0, 0, &huffman::STD_LUMA_DC), (1, 0, &huffman::STD_LUMA_AC)],
        );
        // Hand-written SOS: Ls=6, Ns=0, Ss=0, Se=63, Ah|Al=0.
        ns0.extend_from_slice(&[0xFF, marker::code::SOS, 0x00, 0x06, 0x00, 0x00, 0x3F, 0x00]);
        marker::write_marker(&mut ns0, marker::code::EOI);
        assert!(d(&ns0).is_err(), "SOS Ns=0");
    }
    /// The static message carried by a decode error (`"ok"` for success) — used where a mutant
    /// would change WHICH error is reported, not merely whether one is.
    fn err_msg<T>(r: gamut_core::Result<T>) -> &'static str {
        match r {
            Err(Error::InvalidInput(m)) | Err(Error::Unsupported(m)) => m,
            Err(_) => "other",
            Ok(_) => "ok",
        }
    }

    /// Decodes as Gray8, returning the error message (or `"ok"`).
    fn gray_msg(data: &[u8]) -> &'static str {
        err_msg(<JpegDecoder as DecodeImage<Gray8>>::decode_image(
            &JpegDecoder::new(),
            data,
        ))
    }

    #[test]
    fn soi_validation_is_exact() {
        // A bare SOI passes the SOI check and fails at the NEXT marker read with that read's own
        // message — pinning the `len < 2` boundary (an off-by-one would misreport "missing SOI").
        assert_eq!(gray_msg(&[0xFF, 0xD8]), "JPEG: expected a marker");
        // Each half of the SOI check rejects independently, on an otherwise fully valid stream:
        // right prefix byte + wrong code, and wrong prefix byte + right code. A weakened check
        // would let either stream decode successfully.
        let mut not_soi = gray_stream(8, 8, &[flat(5)], None);
        not_soi[1] = 0xD0;
        assert_eq!(gray_msg(&not_soi), "JPEG: missing SOI marker");
        let mut not_ff = gray_stream(8, 8, &[flat(5)], None);
        not_ff[0] = 0x00;
        assert_eq!(gray_msg(&not_ff), "JPEG: missing SOI marker");
    }

    #[test]
    fn truncation_after_entropy_reports_missing_marker_exactly() {
        // flat(64) codes to exactly 16 bits (cat-7 DC: 5+7, EOB: 4), so the scan consumes its final
        // data byte completely. Stripping the EOI must yield the end-of-scan message — an eagerly
        // over-fetching bit reader (fetching when the buffered bits already suffice) would hit EOF
        // mid-block and misreport truncated entropy data instead.
        let full = gray_stream(8, 8, &[flat(64)], None);
        assert_eq!(
            gray_msg(&full[..full.len() - 2]),
            "JPEG: missing marker after scan"
        );
    }

    #[test]
    fn missing_block_decodes_from_all_ones_padding() {
        // 8×16 gray declares two blocks but the entropy codes only one: the second block is decoded
        // from the reader's 1-padding past the EOI marker (§F.2.2.5) and must fail as an undecodable
        // all-ones Huffman walk. Pins the padding VALUE — corrupted padding (0-bits) would decode
        // spurious category-0 blocks and succeed.
        let (ldc, lac, ..) = std_enc();
        let mut jpeg = Vec::new();
        marker::write_marker(&mut jpeg, marker::code::SOI);
        quant::emit_dqt(&mut jpeg, &[(0, &dc8_quant())]);
        marker::write_sof0(&mut jpeg, 8, 16, &[(1, 1, 1, 0)]);
        huffman::emit_dht(
            &mut jpeg,
            &[(0, 0, &huffman::STD_LUMA_DC), (1, 0, &huffman::STD_LUMA_AC)],
        );
        marker::write_sos(&mut jpeg, &[(1, 0, 0)]);
        jpeg.extend_from_slice(&entropy(&[(0, flat(7))], &[(ldc, lac)]));
        marker::write_marker(&mut jpeg, marker::code::EOI);
        assert_eq!(gray_msg(&jpeg), "JPEG: undecodable Huffman code");
    }

    /// Searches for a flat-DC + tail-coefficient block whose entropy segment ends in a stuffed
    /// `0xFF 0x00` pair (the 1-padding completes the final byte to `0xFF`, which the writer stuffs).
    /// Returns the block; panics if no combination in the small search space aligns.
    fn block_with_stuffed_tail() -> [i32; 64] {
        let (ldc, lac, ..) = std_enc();
        for dc in 0..512 {
            for tail in [1i32, 3, 7, 15, 31] {
                let mut b = [0i32; 64];
                b[0] = dc;
                b[63] = tail; // natural 63 == zig-zag 63: last coefficient, suppresses EOB
                let mut out = Vec::new();
                let mut w = BitWriter::new(&mut out);
                emit_block(&mut w, &b, &mut 0, &ldc, &lac);
                w.flush();
                if out.len() >= 2 && out[out.len() - 2..] == [0xFF, 0x00] {
                    return b;
                }
            }
        }
        panic!("no block aligned to a stuffed 0xFF tail");
    }

    #[test]
    fn dnl_end_detection_survives_stuffed_tail_and_fill_bytes() {
        // A Y=0 frame whose entropy ends in a stuffed 0xFF 0x00 pair, with a fill 0xFF before the
        // DNL marker: the end-of-data peek must byte-align past the stuffed pair, skip the fill, and
        // stop at DNL. This makes the peek's forward lookahead and fill-skip both load-bearing (a
        // backwards peek would see the stuffed 0x00 and keep decoding).
        let block = block_with_stuffed_tail();
        let mut jpeg = gray_stream(8, 0, &[block], Some((4, 8)));
        let dnl = jpeg.windows(2).position(|w| w == [0xFF, 0xDC]).unwrap();
        jpeg.insert(dnl, 0xFF); // fill byte: 0xFF 0xFF 0xDC
        let out: ImageBuf<Gray8> = JpegDecoder::new().decode_image(&jpeg).unwrap();
        assert_eq!(out.dimensions(), gamut_core::Dimensions::new(8, 8).unwrap());
    }

    /// Builds a single-component (gray) 8-wide stream with restart interval 1 (an RSTm between every
    /// MCU row), coding one flat block per row (predictor resets each interval). `y_field` goes in the
    /// SOF `Y`; an optional raw DNL segment (`len`/`nl`) follows the entropy, then EOI.
    fn gray_restart_stream(blocks: &[[i32; 64]], y_field: u16, dnl: Option<(u16, u16)>) -> Vec<u8> {
        let (ldc, lac, ..) = std_enc();
        let mut jpeg = Vec::new();
        marker::write_marker(&mut jpeg, marker::code::SOI);
        quant::emit_dqt(&mut jpeg, &[(0, &dc8_quant())]);
        marker::write_sof0(&mut jpeg, 8, y_field, &[(1, 1, 1, 0)]);
        huffman::emit_dht(
            &mut jpeg,
            &[(0, 0, &huffman::STD_LUMA_DC), (1, 0, &huffman::STD_LUMA_AC)],
        );
        marker::write_dri(&mut jpeg, 1); // restart after every MCU
        marker::write_sos(&mut jpeg, &[(1, 0, 0)]);
        for (i, blk) in blocks.iter().enumerate() {
            let mut seg = Vec::new();
            let mut w = BitWriter::new(&mut seg);
            let mut pred = 0i32; // predictor resets at each restart interval (§E.2.5)
            emit_block(&mut w, blk, &mut pred, &ldc, &lac);
            w.flush();
            jpeg.extend_from_slice(&seg);
            // No trailing restart after the final MCU (a complete scan never emits one).
            if i + 1 < blocks.len() {
                marker::write_marker(&mut jpeg, marker::code::RST0 + (i as u8 & 7));
            }
        }
        if let Some((len, nl)) = dnl {
            marker::write_segment_header(&mut jpeg, marker::code::DNL, usize::from(len));
            jpeg.extend_from_slice(&nl.to_be_bytes());
        }
        marker::write_marker(&mut jpeg, marker::code::EOI);
        jpeg
    }

    #[test]
    fn zero_height_with_restarts_is_not_truncated_at_restart_boundary() {
        // A Y=0 (DNL) frame that uses restart intervals: each RSTm falls exactly on an MCU-row start.
        // The end-of-data heuristic must NOT mistake a restart marker for the end of the scan — a
        // complete scan never emits a trailing RST, so an RSTm always precedes more MCUs. The DNL
        // form (height deferred) must decode identically to the explicit-Y form (no DNL). Before the
        // fix, the Y=0 form truncated after the first block and failed the plane-coverage check.
        let blocks = [flat(10), flat(-30), flat(20)];
        let deferred = gray_restart_stream(&blocks, 0, Some((4, 24)));
        let explicit = gray_restart_stream(&blocks, 24, None);
        let a: ImageBuf<Gray8> = JpegDecoder::new().decode_image(&deferred).unwrap();
        let b: ImageBuf<Gray8> = JpegDecoder::new().decode_image(&explicit).unwrap();
        assert_eq!(a.dimensions(), gamut_core::Dimensions::new(8, 24).unwrap());
        assert_eq!(a, b);
        // Spot-check the three block rows reconstruct to 128 + DC (dc8_quant step 8).
        assert!(a.as_samples()[..64].iter().all(|&s| s == 138)); // 128 + 10
        assert!(a.as_samples()[64..128].iter().all(|&s| s == 98)); // 128 − 30
        assert!(a.as_samples()[128..].iter().all(|&s| s == 148)); // 128 + 20
    }

    #[test]
    fn info_and_decode_classify_stray_markers_exactly() {
        // info() on SOI+EOI (no frame header): the SOS/EOI arm's own message — a deleted arm would
        // fall into the skip-segment path and misreport a truncated segment.
        assert_eq!(
            err_msg(info(&[0xFF, 0xD8, 0xFF, 0xD9])),
            "JPEG: no frame header before scan/end"
        );
        // info() on a standalone TEM after SOI: the standalone-marker arm's own message.
        assert_eq!(
            err_msg(info(&[0xFF, 0xD8, 0xFF, 0x01, 0xFF, 0xD9])),
            "JPEG: unexpected standalone marker"
        );
        // Decode: a TEM outside a scan is the decode loop's own standalone-marker message.
        let mut s = gray_stream(8, 8, &[flat(1)], None);
        let dqt = s.windows(2).position(|w| w == [0xFF, 0xDB]).unwrap();
        s.splice(dqt..dqt, [0xFF, 0x01]);
        assert_eq!(
            gray_msg(&s),
            "JPEG: unexpected standalone marker outside a scan"
        );
    }

    /// Builds a 20×12 4:2:0 stream (luma 2×2, chroma 1×1) whose luma block grid is NOT MCU-aligned
    /// (20 = 1¼ MCUs wide): non-interleaved luma is 3×2 blocks (§A.2.2, ceil(20/8)×ceil(12/8)) while
    /// the interleaved walk is 4×2 (2 MCUs × 2 blocks, partial-MCU completion). `luma(bx, by)` and
    /// `chroma(ci, bx)` supply each block's coefficients by grid position.
    fn ycbcr420_20x12(interleaved: bool) -> Vec<u8> {
        let luma = |bx: i32, by: i32| {
            let mut b = [0i32; 64];
            b[0] = bx * 3 + by * 5 - 4;
            b[1] = bx - by;
            b
        };
        let chroma = |ci: i32, bx: i32| {
            let mut b = [0i32; 64];
            b[0] = ci * 6 - 3 + bx;
            b
        };
        let (ldc, lac, cdc, cac) = std_enc();
        let mut jpeg = Vec::new();
        marker::write_marker(&mut jpeg, marker::code::SOI);
        marker::write_app0_jfif(&mut jpeg, marker::DensityUnit::AspectRatio, 1, 1);
        quant::emit_dqt(&mut jpeg, &[(0, &[1u8; 64]), (1, &[1u8; 64])]);
        marker::write_sof0(
            &mut jpeg,
            20,
            12,
            &[(1, 2, 2, 0), (2, 1, 1, 1), (3, 1, 1, 1)],
        );
        huffman::emit_dht(
            &mut jpeg,
            &[
                (0, 0, &huffman::STD_LUMA_DC),
                (1, 0, &huffman::STD_LUMA_AC),
                (0, 1, &huffman::STD_CHROMA_DC),
                (1, 1, &huffman::STD_CHROMA_AC),
            ],
        );
        if interleaved {
            marker::write_sos(&mut jpeg, &[(1, 0, 0), (2, 1, 1), (3, 1, 1)]);
            // 2×1 MCUs; per MCU: luma (2×2 blocks row-major), then Cb, then Cr (§A.2.3).
            let mut order = Vec::new();
            for mx in 0..2i32 {
                for by in 0..2i32 {
                    for bx in 0..2i32 {
                        order.push((0usize, luma(mx * 2 + bx, by)));
                    }
                }
                order.push((1, chroma(1, mx)));
                order.push((2, chroma(2, mx)));
            }
            let tables = [
                (ldc.clone(), lac.clone()),
                (cdc.clone(), cac.clone()),
                (cdc, cac),
            ];
            jpeg.extend_from_slice(&entropy(&order, &tables));
        } else {
            // Luma scan: 3×2 blocks over the COMPONENT's own grid, row-major (§A.2.2).
            marker::write_sos(&mut jpeg, &[(1, 0, 0)]);
            let mut order = Vec::new();
            for by in 0..2i32 {
                for bx in 0..3i32 {
                    order.push((0usize, luma(bx, by)));
                }
            }
            jpeg.extend_from_slice(&entropy(&order, &[(ldc, lac)]));
            // Chroma scans: comp 10×6 → 2×1 blocks each.
            marker::write_sos(&mut jpeg, &[(2, 1, 1)]);
            let cb: Vec<(usize, [i32; 64])> = (0..2).map(|bx| (0usize, chroma(1, bx))).collect();
            jpeg.extend_from_slice(&entropy(&cb, &[(cdc.clone(), cac.clone())]));
            marker::write_sos(&mut jpeg, &[(3, 1, 1)]);
            let cr: Vec<(usize, [i32; 64])> = (0..2).map(|bx| (0usize, chroma(2, bx))).collect();
            jpeg.extend_from_slice(&entropy(&cr, &[(cdc, cac)]));
        }
        marker::write_marker(&mut jpeg, marker::code::EOI);
        jpeg
    }

    #[test]
    fn non_aligned_420_non_interleaved_matches_interleaved() {
        // 20×12 4:2:0: the luma block grid is not MCU-aligned, so a decoder that walked a
        // non-interleaved scan with the interleaved MCU geometry would consume the wrong number of
        // blocks (8 vs 6) and misplace every sample. The two encodings of the same content must
        // decode identically over the visible region.
        let a: ImageBuf<Rgb8> = JpegDecoder::new()
            .decode_image(&ycbcr420_20x12(true))
            .unwrap();
        let b: ImageBuf<Rgb8> = JpegDecoder::new()
            .decode_image(&ycbcr420_20x12(false))
            .unwrap();
        assert_eq!(a.dimensions(), gamut_core::Dimensions::new(20, 12).unwrap());
        assert_eq!(a, b);
    }
}
