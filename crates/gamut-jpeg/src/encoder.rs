//! The baseline sequential DCT Huffman encoder: [`JpegEncoder`] and its pipeline.
//!
//! For each 8×8 block the pipeline is T.81 Annex A end to end — colour-convert (T.871 §7) →
//! chroma-subsample → level shift (§A.3.1) → forward DCT (§A.3.3, via `gamut_dsp`) → quantize
//! (§A.3.4) → zig-zag (§A.3.6) → differential DC + run-length AC Huffman coding (Annex F §F.1.2) —
//! interleaved into minimum coded units (§A.2.3) and wrapped in a JFIF interchange stream (§B.2).

use gamut_color::{ColorRange, rgb_to_ycbcr};
use gamut_core::{Dimensions, EncodeImage, Error, Gray8, ImageRef, Result, Rgb8};
use gamut_dsp::jpeg::fdct8x8;
use gamut_dsp::math::round_div_nearest;

use crate::bitwriter::BitWriter;
use crate::huffman::{self, EncTable, TableSpec};
use crate::marker::{self, DensityUnit};
use crate::zigzag::ZIGZAG;
use crate::{progressive, quant};

/// The largest image dimension the frame header can encode: the SOF0 `X`/`Y` fields are 16-bit
/// (§B.2.2, Table B.2).
const MAX_DIMENSION: u32 = u16::MAX as u32;

/// Chroma subsampling mode for YCbCr (colour) encoding: the ratio at which the Cb/Cr planes are
/// sampled relative to luma. Ignored for grayscale, which has a single component.
///
/// Named for the conventional `J:a:b` notation. The luma sampling factors are `1×1` (4:4:4),
/// `2×1` (4:2:2), or `2×2` (4:2:0); chroma is always `1×1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ChromaSubsampling {
    /// 4:4:4 — no chroma subsampling; full-resolution Cb/Cr (largest files, best chroma fidelity).
    Ycbcr444,
    /// 4:2:2 — Cb/Cr subsampled 2:1 horizontally only.
    Ycbcr422,
    /// 4:2:0 — Cb/Cr subsampled 2:1 both horizontally and vertically (the common photographic
    /// default; T.871 §9 NOTE 3 names it the most common form).
    Ycbcr420,
}

impl ChromaSubsampling {
    /// The luma horizontal/vertical sampling factors `(Hy, Vy)`; chroma is fixed at `1×1`, so these
    /// double as the box-subsampling factors applied to each chroma plane.
    fn luma_factors(self) -> (u8, u8) {
        match self {
            ChromaSubsampling::Ycbcr444 => (1, 1),
            ChromaSubsampling::Ycbcr422 => (2, 1),
            ChromaSubsampling::Ycbcr420 => (2, 2),
        }
    }
}

/// A reusable baseline JPEG encoder.
///
/// Configure it with the builder methods, then drive it through [`EncodeImage`]. It writes JFIF
/// interchange streams: grayscale ([`Gray8`], one component) or YCbCr ([`Rgb8`], converted per
/// T.871 §7 with the configured [`ChromaSubsampling`]).
///
/// # Frozen quality contract
///
/// For a given `(quality, subsampling)` the quantization tables — and therefore the coefficient
/// values and byte stream — are SemVer-stable: quality 50 emits the T.81 Annex K tables verbatim,
/// and the IJG quality→scale mapping is frozen.
///
/// # Example
///
/// ```
/// use gamut_core::{Dimensions, EncodeImage, ImageRef, Gray8};
/// use gamut_jpeg::JpegEncoder;
///
/// let pixels = vec![128u8; 8 * 8];
/// let image = ImageRef::<Gray8>::new(&pixels, Dimensions::new(8, 8)?)?;
/// let mut jpeg = Vec::new();
/// JpegEncoder::new().with_quality(90).encode_image(image, &mut jpeg)?;
/// assert_eq!(&jpeg[..2], &[0xFF, 0xD8]); // SOI
/// # Ok::<(), gamut_core::Error>(())
/// ```
#[derive(Debug, Clone)]
pub struct JpegEncoder {
    quality: u8,
    subsampling: ChromaSubsampling,
    restart_interval: u16,
    density_unit: DensityUnit,
    x_density: u16,
    y_density: u16,
    progressive: bool,
}

impl Default for JpegEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl JpegEncoder {
    /// Creates an encoder with quality 75, [`ChromaSubsampling::Ycbcr420`], no restart interval, and
    /// a 1:1 aspect-ratio pixel density (JFIF `units = 0`).
    #[must_use]
    pub fn new() -> Self {
        Self {
            quality: 75,
            subsampling: ChromaSubsampling::Ycbcr420,
            restart_interval: 0,
            density_unit: DensityUnit::AspectRatio,
            x_density: 1,
            y_density: 1,
            progressive: false,
        }
    }

    /// Sets the quality, **clamped** to `1..=100` (higher is better/larger). Quality 50 uses the
    /// Annex K tables verbatim; 100 uses all-1 tables. Clamping (rather than rejecting) matches
    /// libjpeg's `jpeg_set_quality`.
    #[must_use]
    pub fn with_quality(mut self, quality: u8) -> Self {
        self.quality = quality.clamp(1, 100);
        self
    }

    /// Sets the chroma [`ChromaSubsampling`] used for colour ([`Rgb8`]) input. No effect on
    /// grayscale.
    #[must_use]
    pub fn with_subsampling(mut self, subsampling: ChromaSubsampling) -> Self {
        self.subsampling = subsampling;
        self
    }

    /// Sets the restart interval in MCUs: a restart marker (RSTn) is inserted every `mcus` MCUs,
    /// letting a decoder resynchronize. `0` (the default) disables restarts, emitting no DRI segment.
    #[must_use]
    pub fn with_restart_interval(mut self, mcus: u16) -> Self {
        self.restart_interval = mcus;
        self
    }

    /// Sets the JFIF pixel density written to the APP0 segment: the [`DensityUnit`] and the
    /// horizontal/vertical densities. Each density is clamped to be non-zero, as T.871 §10.1
    /// requires.
    #[must_use]
    pub fn with_density(mut self, unit: DensityUnit, x_density: u16, y_density: u16) -> Self {
        self.density_unit = unit;
        self.x_density = x_density.max(1);
        self.y_density = y_density.max(1);
        self
    }

    /// Selects the **progressive DCT** process (SOF2, T.81 Annex G) when `true`, or the default
    /// baseline sequential process (SOF0) when `false`.
    ///
    /// A progressive stream codes the image as several scans, each carrying one spectral band at one
    /// successive-approximation precision, so a decoder can render a coarse whole-image preview from
    /// the first scans and refine it as more arrive. gamut uses libjpeg's frozen
    /// `jpeg_simple_progression` scan script (a 6-scan gray / 10-scan YCbCr layout) with optimized
    /// per-scan Huffman tables (Annex K.2). The quantized coefficients — and therefore the decoded
    /// image — are identical to the baseline encoding of the same input at the same
    /// `(quality, subsampling)`; only the stream structure differs.
    #[must_use]
    pub fn with_progressive(mut self, progressive: bool) -> Self {
        self.progressive = progressive;
        self
    }

    /// The scaled luminance quantization table (natural order) for the configured quality.
    fn luma_quant(&self) -> [u8; 64] {
        quant::scale(&quant::LUMINANCE, self.quality)
    }

    /// The scaled chrominance quantization table (natural order) for the configured quality.
    fn chroma_quant(&self) -> [u8; 64] {
        quant::scale(&quant::CHROMINANCE, self.quality)
    }

    /// Rejects dimensions the frame header cannot encode (`X`/`Y` are 16-bit). Zero is already
    /// excluded by [`Dimensions`].
    fn check_dimensions(dims: Dimensions) -> Result<(u16, u16)> {
        if dims.width > MAX_DIMENSION || dims.height > MAX_DIMENSION {
            return Err(Error::InvalidInput("JPEG: image exceeds 65535×65535"));
        }
        Ok((dims.width as u16, dims.height as u16))
    }

    /// Writes the leading markers common to every stream: SOI, JFIF APP0, and the DQT segment.
    fn write_prologue(&self, out: &mut Vec<u8>, quant_tables: &[(u8, &[u8; 64])]) {
        marker::write_marker(out, marker::code::SOI);
        marker::write_app0_jfif(out, self.density_unit, self.x_density, self.y_density);
        quant::emit_dqt(out, quant_tables);
    }
}

/// A single-channel sample plane at a component's own resolution (row-major, 8-bit).
pub(crate) struct Plane {
    pub(crate) data: Vec<u8>,
    pub(crate) width: usize,
    pub(crate) height: usize,
}

impl Plane {
    /// The sample at `(x, y)` with edge replication (clamping past the plane bounds), level-shifted
    /// to the signed baseline range by subtracting 128 (§A.3.1, `P = 8`). Edge replication is the
    /// encoder's free choice for padding partial edge blocks/MCUs to a whole 8×8 (§A.2.3): repeating
    /// the border minimizes spurious high-frequency energy versus zero-fill.
    pub(crate) fn level_shifted(&self, x: usize, y: usize) -> i32 {
        let cx = x.min(self.width - 1);
        let cy = y.min(self.height - 1);
        i32::from(self.data[cy * self.width + cx]) - 128
    }
}

/// One frame component paired with the tables and sampling used to code it.
struct Component<'a> {
    /// Horizontal sampling factor `Hi`.
    h: u8,
    /// Vertical sampling factor `Vi`.
    v: u8,
    plane: &'a Plane,
    quant: &'a [u8; 64],
    dc: &'a EncTable,
    ac: &'a EncTable,
}

/// The magnitude category `SSSS` of `value` (Annex F §F.1.2): the number of bits needed for
/// `|value|`, and `0` for `value == 0`.
pub(crate) fn magnitude_category(value: i32) -> u8 {
    (32 - value.unsigned_abs().leading_zeros()) as u8
}

/// The `SSSS` additional bits appended after a DC/AC Huffman code (Annex F §F.1.2.1): the low
/// `category` bits of `value` for a positive value, or of `value - 1` (the "one lower precision"
/// negative encoding) for a negative value.
pub(crate) fn additional_bits(value: i32, category: u8) -> u16 {
    let v = if value < 0 { value - 1 } else { value };
    (v as u32 & ((1u32 << category) - 1)) as u16
}

/// Emits the Huffman code for `symbol` from `table`. The entropy coder only ever produces symbols
/// present in the standard tables (DC categories 0..=11; AC run/size, EOB `0x00`, ZRL `0xF0`), so a
/// missing symbol is a logic error, asserted in debug builds.
fn emit_symbol(writer: &mut BitWriter, table: &EncTable, symbol: u8) {
    match table.lookup(symbol) {
        Some((code, length)) => writer.write_bits(code, length),
        None => debug_assert!(false, "Huffman symbol {symbol:#x} absent from table"),
    }
}

/// Level-shifts, forward-transforms and quantizes one 8×8 block of `plane` at block coordinates
/// `(bx, by)` (§A.3.1 / §A.3.3 / §A.3.4), returning the natural-order quantized coefficients. Shared
/// by the baseline single-pass coder ([`encode_block`]) and the progressive encoder
/// ([`crate::progressive`]), which materializes every block up front before running the scan script.
pub(crate) fn quantize_block(plane: &Plane, quant: &[u8; 64], bx: usize, by: usize) -> [i32; 64] {
    // Gather the level-shifted samples in natural (raster) order.
    let mut block = [0i32; 64];
    for row in 0..8usize {
        for col in 0..8usize {
            block[row * 8 + col] = plane.level_shifted(bx * 8 + col, by * 8 + row);
        }
    }
    fdct8x8(&mut block);
    // Quantize (§A.3.4): round-to-nearest divide by the table entry (which is ≥ 1).
    let mut q = [0i32; 64];
    for (dst, (&coeff, &step)) in q.iter_mut().zip(block.iter().zip(quant.iter())) {
        *dst = round_div_nearest(coeff, i32::from(step));
    }
    q
}

/// Codes one 8×8 block (§A.3): level-shift → FDCT → quantize, then hands the natural-order
/// quantized coefficients to [`encode_quantized_block`] for entropy coding.
fn encode_block(
    comp: &Component,
    block_x: usize,
    block_y: usize,
    dc_pred: &mut i32,
    writer: &mut BitWriter,
) {
    let q = quantize_block(comp.plane, comp.quant, block_x, block_y);
    encode_quantized_block(&q, dc_pred, comp.dc, comp.ac, writer);
}

/// Entropy-codes one block of quantized coefficients (natural order) per §F.1.2: the DC difference
/// against the running predictor (§F.1.2.1, updating it), then the run-length AC symbols in zig-zag
/// order (§F.1.2.2) — ZRL for zero runs of 16, EOB unless the last zig-zag coefficient is nonzero.
fn encode_quantized_block(
    q: &[i32; 64],
    dc_pred: &mut i32,
    dc: &EncTable,
    ac: &EncTable,
    writer: &mut BitWriter,
) {
    // DC: differential coding against the running predictor (§F.1.2.1).
    let diff = q[0] - *dc_pred;
    *dc_pred = q[0];
    let cat = magnitude_category(diff);
    emit_symbol(writer, dc, cat);
    writer.write_bits(additional_bits(diff, cat), cat);

    // AC: run-length of zeros then (run, size) symbols in zig-zag order (§F.1.2.2).
    let mut run = 0u8;
    for &natural in &ZIGZAG[1..] {
        let coeff = q[natural];
        if coeff == 0 {
            run += 1;
            continue;
        }
        while run >= 16 {
            emit_symbol(writer, ac, 0xF0); // ZRL: 16 zeros
            run -= 16;
        }
        let cat = magnitude_category(coeff);
        emit_symbol(writer, ac, marker::pack_nibbles(run, cat));
        writer.write_bits(additional_bits(coeff, cat), cat);
        run = 0;
    }
    if run > 0 {
        emit_symbol(writer, ac, 0x00); // EOB: block ends in zeros
    }
}

/// Codes the interleaved scan over all components (§A.2.3): walk MCUs row-major, and within each MCU
/// walk each component's `Vi×Hi` blocks. Restart markers are inserted every `restart_interval` MCUs
/// (predictors reset), and the final entropy byte is padded before EOI. A single-component (gray)
/// scan degenerates to one 8×8 block per MCU — the non-interleaved order of §A.2.2.
fn encode_scan(
    components: &[Component],
    width: u32,
    height: u32,
    restart_interval: u16,
    out: &mut Vec<u8>,
) {
    let hmax = components.iter().map(|c| c.h).max().unwrap_or(1);
    let vmax = components.iter().map(|c| c.v).max().unwrap_or(1);
    let mcu_w = 8 * u32::from(hmax);
    let mcu_h = 8 * u32::from(vmax);
    let mcus_x = width.div_ceil(mcu_w);
    let mcus_y = height.div_ceil(mcu_h);

    let mut writer = BitWriter::new(out);
    let mut dc_pred = vec![0i32; components.len()];
    let mut mcu_index = 0u32;
    let mut restart_m = 0u8;

    for my in 0..mcus_y {
        for mx in 0..mcus_x {
            if restart_interval != 0
                && mcu_index != 0
                && mcu_index.is_multiple_of(u32::from(restart_interval))
            {
                writer.restart(restart_m);
                restart_m = restart_m.wrapping_add(1);
                dc_pred.iter_mut().for_each(|p| *p = 0);
            }
            for (ci, comp) in components.iter().enumerate() {
                for by in 0..u32::from(comp.v) {
                    for bx in 0..u32::from(comp.h) {
                        let block_x = (mx * u32::from(comp.h) + bx) as usize;
                        let block_y = (my * u32::from(comp.v) + by) as usize;
                        encode_block(comp, block_x, block_y, &mut dc_pred[ci], &mut writer);
                    }
                }
            }
            mcu_index += 1;
        }
    }
    writer.flush();
}

/// Box-averages `plane` (row-major, `width`×`height`) by `(sx, sy)`, producing a
/// `ceil(width/sx)`×`ceil(height/sy)` plane. Partial edge boxes average only the samples that
/// exist (equivalent to edge replication). With `sx == sy == 1` this is an exact copy (4:4:4).
///
/// Box averaging is the encoder's documented free choice; T.81 leaves the subsampling filter open,
/// and T.871 §9 NOTE 1 suggests a simple two-tap `(½, ½)` filter for 2:1.
fn downsample(plane: &[u8], width: usize, height: usize, sx: usize, sy: usize) -> Plane {
    let cw = width.div_ceil(sx);
    let ch = height.div_ceil(sy);
    let mut data = vec![0u8; cw * ch];
    for cy in 0..ch {
        for cx in 0..cw {
            let (mut sum, mut count) = (0u32, 0u32);
            for dy in 0..sy {
                for dx in 0..sx {
                    let px = cx * sx + dx;
                    let py = cy * sy + dy;
                    if px < width && py < height {
                        sum += u32::from(plane[py * width + px]);
                        count += 1;
                    }
                }
            }
            data[cy * cw + cx] = ((sum + count / 2) / count) as u8;
        }
    }
    Plane {
        data,
        width: cw,
        height: ch,
    }
}

impl EncodeImage<Gray8> for JpegEncoder {
    /// Encodes a grayscale image as a single-component (Y) baseline JPEG. Subsampling does not apply
    /// to a one-component image; a JFIF APP0 segment is still written.
    fn encode_image(&self, image: ImageRef<'_, Gray8>, out: &mut Vec<u8>) -> Result<usize> {
        let (width, height) = Self::check_dimensions(image.dimensions())?;
        let start = out.len();

        let plane = Plane {
            data: image.as_samples().to_vec(),
            width: usize::from(width),
            height: usize::from(height),
        };
        let luma_quant = self.luma_quant();

        self.write_prologue(out, &[(0, &luma_quant)]);
        if self.progressive {
            let comps = [progressive::ProgComponent {
                id: 1,
                h: 1,
                v: 1,
                tq: 0,
                plane: &plane,
                quant: &luma_quant,
            }];
            progressive::encode(out, width, height, &comps, self.restart_interval);
        } else {
            let dc = EncTable::from_spec(&huffman::STD_LUMA_DC);
            let ac = EncTable::from_spec(&huffman::STD_LUMA_AC);
            marker::write_sof0(out, width, height, &[(1, 1, 1, 0)]);
            emit_huffman_tables(out, false);
            if self.restart_interval != 0 {
                marker::write_dri(out, self.restart_interval);
            }
            marker::write_sos(out, &[(1, 0, 0)]);

            let comp = Component {
                h: 1,
                v: 1,
                plane: &plane,
                quant: &luma_quant,
                dc: &dc,
                ac: &ac,
            };
            encode_scan(
                &[comp],
                u32::from(width),
                u32::from(height),
                self.restart_interval,
                out,
            );
        }

        marker::write_marker(out, marker::code::EOI);
        Ok(out.len() - start)
    }
}

impl EncodeImage<Rgb8> for JpegEncoder {
    /// Encodes an RGB image as a three-component YCbCr baseline JPEG: RGB is converted to full-range
    /// (JFIF) BT.601 YCbCr per T.871 §7, and the chroma planes are subsampled per the configured
    /// [`ChromaSubsampling`].
    fn encode_image(&self, image: ImageRef<'_, Rgb8>, out: &mut Vec<u8>) -> Result<usize> {
        let (width, height) = Self::check_dimensions(image.dimensions())?;
        let start = out.len();
        let (w, h) = (usize::from(width), usize::from(height));

        // RGB → full-resolution Y/Cb/Cr planes (T.871 §7 full-range BT.601, fixed-point).
        let rgb = image.as_samples();
        let mut y = vec![0u8; w * h];
        let mut cb = vec![0u8; w * h];
        let mut cr = vec![0u8; w * h];
        for i in 0..w * h {
            let (yy, u, v) =
                rgb_to_ycbcr(rgb[i * 3], rgb[i * 3 + 1], rgb[i * 3 + 2], ColorRange::Full);
            y[i] = yy;
            cb[i] = u;
            cr[i] = v;
        }
        let (yh, yv) = self.subsampling.luma_factors();
        let (sx, sy) = (usize::from(yh), usize::from(yv));
        let luma_plane = Plane {
            data: y,
            width: w,
            height: h,
        };
        let cb_plane = downsample(&cb, w, h, sx, sy);
        let cr_plane = downsample(&cr, w, h, sx, sy);

        let luma_quant = self.luma_quant();
        let chroma_quant = self.chroma_quant();

        self.write_prologue(out, &[(0, &luma_quant), (1, &chroma_quant)]);
        if self.progressive {
            let comps = [
                progressive::ProgComponent {
                    id: 1,
                    h: yh,
                    v: yv,
                    tq: 0,
                    plane: &luma_plane,
                    quant: &luma_quant,
                },
                progressive::ProgComponent {
                    id: 2,
                    h: 1,
                    v: 1,
                    tq: 1,
                    plane: &cb_plane,
                    quant: &chroma_quant,
                },
                progressive::ProgComponent {
                    id: 3,
                    h: 1,
                    v: 1,
                    tq: 1,
                    plane: &cr_plane,
                    quant: &chroma_quant,
                },
            ];
            progressive::encode(out, width, height, &comps, self.restart_interval);
        } else {
            let luma_dc = EncTable::from_spec(&huffman::STD_LUMA_DC);
            let luma_ac = EncTable::from_spec(&huffman::STD_LUMA_AC);
            let chroma_dc = EncTable::from_spec(&huffman::STD_CHROMA_DC);
            let chroma_ac = EncTable::from_spec(&huffman::STD_CHROMA_AC);

            marker::write_sof0(
                out,
                width,
                height,
                &[(1, yh, yv, 0), (2, 1, 1, 1), (3, 1, 1, 1)],
            );
            emit_huffman_tables(out, true);
            if self.restart_interval != 0 {
                marker::write_dri(out, self.restart_interval);
            }
            marker::write_sos(out, &[(1, 0, 0), (2, 1, 1), (3, 1, 1)]);

            let components = [
                Component {
                    h: yh,
                    v: yv,
                    plane: &luma_plane,
                    quant: &luma_quant,
                    dc: &luma_dc,
                    ac: &luma_ac,
                },
                Component {
                    h: 1,
                    v: 1,
                    plane: &cb_plane,
                    quant: &chroma_quant,
                    dc: &chroma_dc,
                    ac: &chroma_ac,
                },
                Component {
                    h: 1,
                    v: 1,
                    plane: &cr_plane,
                    quant: &chroma_quant,
                    dc: &chroma_dc,
                    ac: &chroma_ac,
                },
            ];
            encode_scan(
                &components,
                u32::from(width),
                u32::from(height),
                self.restart_interval,
                out,
            );
        }

        marker::write_marker(out, marker::code::EOI);
        Ok(out.len() - start)
    }
}

/// Emits the DHT segment for a scan: luma DC/AC (destinations 0) always, plus chroma DC/AC
/// (destinations 1) when `color`.
fn emit_huffman_tables(out: &mut Vec<u8>, color: bool) {
    let luma: [(u8, u8, &TableSpec); 2] =
        [(0, 0, &huffman::STD_LUMA_DC), (1, 0, &huffman::STD_LUMA_AC)];
    if color {
        huffman::emit_dht(
            out,
            &[
                luma[0],
                luma[1],
                (0, 1, &huffman::STD_CHROMA_DC),
                (1, 1, &huffman::STD_CHROMA_AC),
            ],
        );
    } else {
        huffman::emit_dht(out, &luma);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_new() {
        // `Default` must equal `new()`'s configuration: quality 75, 4:2:0, no restart, 1:1 aspect.
        let d = JpegEncoder::default();
        assert_eq!(d.quality, 75);
        assert_eq!(d.subsampling, ChromaSubsampling::Ycbcr420);
        assert_eq!(d.restart_interval, 0);
        assert_eq!(d.density_unit, DensityUnit::AspectRatio);
        assert_eq!((d.x_density, d.y_density), (1, 1));
        assert!(!d.progressive);
    }

    #[test]
    fn with_progressive_toggles_the_flag() {
        assert!(JpegEncoder::new().with_progressive(true).progressive);
        assert!(
            !JpegEncoder::new()
                .with_progressive(true)
                .with_progressive(false)
                .progressive
        );
    }

    #[test]
    fn magnitude_category_matches_f_1_2() {
        // Category = bit length of the magnitude; 0 → 0 (F.1.2). Boundaries pin the ">> until zero".
        assert_eq!(magnitude_category(0), 0);
        assert_eq!(magnitude_category(1), 1);
        assert_eq!(magnitude_category(-1), 1);
        assert_eq!(magnitude_category(2), 2);
        assert_eq!(magnitude_category(-2), 2);
        assert_eq!(magnitude_category(7), 3);
        assert_eq!(magnitude_category(-8), 4);
        assert_eq!(magnitude_category(1023), 10);
        assert_eq!(magnitude_category(2047), 11);
    }

    #[test]
    fn additional_bits_positive_and_negative() {
        // Positive: the value's own low bits. Negative: (value − 1)'s low bits (the F.1.2.1 "one
        // lower precision" complement). For category 3: +5 → 0b101 = 5; −5 → (−6) & 0b111 = 0b010.
        assert_eq!(additional_bits(5, 3), 0b101);
        assert_eq!(additional_bits(-5, 3), 0b010);
        // +1 → 1, −1 → 0 (category 1): the canonical smallest pair.
        assert_eq!(additional_bits(1, 1), 1);
        assert_eq!(additional_bits(-1, 1), 0);
        // Category 0 (DC diff of 0) yields no bits.
        assert_eq!(additional_bits(0, 0), 0);
        // Zero is *non-negative*: its own low bits (0), not the negative complement — pins the
        // strict `< 0` test (a `<= 0` mutant would take the −1 branch and yield 0b111).
        assert_eq!(additional_bits(0, 3), 0);
    }

    #[test]
    fn downsample_444_is_identity() {
        let src = [10u8, 20, 30, 40];
        let p = downsample(&src, 2, 2, 1, 1);
        assert_eq!((p.width, p.height), (2, 2));
        assert_eq!(p.data, src);
    }

    #[test]
    fn downsample_420_box_averages_with_rounding() {
        // A 2×2 plane → one sample = round((10+20+30+40)/4) = round(25) = 25.
        let src = [10u8, 20, 30, 40];
        let p = downsample(&src, 2, 2, 2, 2);
        assert_eq!((p.width, p.height), (1, 1));
        assert_eq!(p.data, vec![25]);
        // Odd width → partial edge box averages only existing samples (edge replication).
        // 3×1 plane, sx=2: box0 = round((10+20)/2)=15, box1 = just 30.
        let odd = [10u8, 20, 30];
        let q = downsample(&odd, 3, 1, 2, 1);
        assert_eq!((q.width, q.height), (2, 1));
        assert_eq!(q.data, vec![15, 30]);
    }

    #[test]
    fn downsample_vertical_only() {
        // A 1×4 column with (sx, sy) = (1, 2): box 0 = rows 0–1 → round((10+20)/2) = 15, box 1 =
        // rows 2–3 → round((30+40)/2) = 35. A height-1 output cannot see a broken `cy·sy` source
        // row; box 1 (cy = 1) pins it — a `cy/sy` mutant re-reads rows 0–1 and yields 15, not 35.
        let src = [10u8, 20, 30, 40];
        let p = downsample(&src, 1, 4, 1, 2);
        assert_eq!((p.width, p.height), (1, 2));
        assert_eq!(p.data, vec![15, 35]);
    }

    #[test]
    fn quant_tables_scale_with_quality() {
        // The encoder's per-component tables are the Annex K bases through the frozen IJG mapping —
        // never a placeholder. Anchor entries computed by hand: at q=75 (scale 50),
        // luminance[0]=16 → (16·50+50)/100 = 8; chrominance[0]=17 → (17·50+50)/100 = 9.
        let e = JpegEncoder::new().with_quality(75);
        assert_eq!(e.luma_quant(), quant::scale(&quant::LUMINANCE, 75));
        assert_eq!(e.chroma_quant(), quant::scale(&quant::CHROMINANCE, 75));
        assert_eq!(e.luma_quant()[0], 8);
        assert_eq!(e.chroma_quant()[0], 9);
    }

    #[test]
    fn level_shift_clamps_and_subtracts_128() {
        let plane = Plane {
            data: vec![200u8, 100, 50, 0],
            width: 2,
            height: 2,
        };
        assert_eq!(plane.level_shifted(0, 0), 200 - 128);
        assert_eq!(plane.level_shifted(1, 1), 0 - 128);
        // Past the right/bottom edge replicates the border sample (index 3 = 0 → −128).
        assert_eq!(plane.level_shifted(5, 5), 0 - 128);
        assert_eq!(plane.level_shifted(0, 9), 50 - 128); // clamps y to row 1, col 0
    }

    // --- An in-crate entropy decoder: decodes a scan produced by `encode_scan` back to quantized
    // coefficients, so the DC-difference / AC run-length invariants can be asserted crisply. It is
    // the inverse of the F.1.2 coder and pins it against a family of encode-side mutants. ---

    /// A de-stuffing, MSB-first bit reader over an entropy-coded segment (no restart markers).
    struct BitReader<'a> {
        bytes: &'a [u8],
        pos: usize,
        bit: u8,
    }

    impl BitReader<'_> {
        fn read_bit(&mut self) -> u32 {
            let byte = self.bytes[self.pos];
            let out = u32::from((byte >> (7 - self.bit)) & 1);
            self.bit += 1;
            if self.bit == 8 {
                self.bit = 0;
                self.pos += 1;
                if byte == 0xFF {
                    self.pos += 1; // skip the stuffed 0x00
                }
            }
            out
        }

        fn read_bits(&mut self, n: u8) -> u32 {
            let mut v = 0;
            for _ in 0..n {
                v = (v << 1) | self.read_bit();
            }
            v
        }

        /// Decodes one Huffman symbol against the inverted `(code, len, symbol)` list.
        fn decode_symbol(&mut self, table: &[(u16, u8, u8)]) -> u8 {
            let mut code = 0u16;
            for len in 1..=16u8 {
                code = (code << 1) | self.read_bit() as u16;
                if let Some(&(_, _, sym)) = table.iter().find(|&&(c, l, _)| l == len && c == code) {
                    return sym;
                }
            }
            panic!("no Huffman symbol matched");
        }

        /// Decodes an `SSSS`-bit signed magnitude value (the inverse of [`additional_bits`]).
        fn decode_value(&mut self, category: u8) -> i32 {
            if category == 0 {
                return 0;
            }
            let raw = self.read_bits(category) as i32;
            // Top bit 0 ⇒ negative branch: value = raw − (2^cat − 1).
            if raw < (1 << (category - 1)) {
                raw - ((1 << category) - 1)
            } else {
                raw
            }
        }
    }

    fn invert(table: &EncTable) -> Vec<(u16, u8, u8)> {
        (0..=255u16)
            .filter_map(|s| table.lookup(s as u8).map(|(c, l)| (c, l, s as u8)))
            .collect()
    }

    /// Decodes `block_count` sequential blocks (one component, one table pair), returning each
    /// block's `(dc_diff, natural-order quantized coefficients)`.
    fn decode_blocks(
        entropy: &[u8],
        dc: &EncTable,
        ac: &EncTable,
        block_count: usize,
    ) -> Vec<(i32, [i32; 64])> {
        let dc_tab = invert(dc);
        let ac_tab = invert(ac);
        let mut reader = BitReader {
            bytes: entropy,
            pos: 0,
            bit: 0,
        };
        let mut blocks = Vec::new();
        for _ in 0..block_count {
            let mut coeffs = [0i32; 64];
            let dc_cat = reader.decode_symbol(&dc_tab);
            let dc_diff = reader.decode_value(dc_cat);
            coeffs[0] = dc_diff; // caller resolves the running DC prediction
            let mut k = 1usize;
            while k < 64 {
                let rs = reader.decode_symbol(&ac_tab);
                let (run, size) = (rs >> 4, rs & 0x0F);
                if size == 0 {
                    if run == 15 {
                        k += 16; // ZRL
                        continue;
                    }
                    break; // EOB
                }
                k += run as usize;
                coeffs[ZIGZAG[k]] = reader.decode_value(size);
                k += 1;
            }
            blocks.push((dc_diff, coeffs));
        }
        blocks
    }

    #[test]
    fn constant_image_dc_predicts_and_ac_is_empty() {
        // A 16×16 constant plane is four identical 8×8 Y blocks. The first block carries the full DC
        // difference from the zero predictor; the next three predict perfectly, so their DC diff is
        // exactly 0 (category-0 code, no magnitude bits) — the observable §F.1.2.1 prediction. Every
        // block's AC is empty (immediate EOB).
        let plane = Plane {
            data: vec![200u8; 16 * 16],
            width: 16,
            height: 16,
        };
        let quant = quant::scale(&quant::LUMINANCE, 50);
        let dc = EncTable::from_spec(&huffman::STD_LUMA_DC);
        let ac = EncTable::from_spec(&huffman::STD_LUMA_AC);
        let comp = Component {
            h: 1,
            v: 1,
            plane: &plane,
            quant: &quant,
            dc: &dc,
            ac: &ac,
        };

        let mut entropy = Vec::new();
        encode_scan(&[comp], 16, 16, 0, &mut entropy);
        let blocks = decode_blocks(&entropy, &dc, &ac, 4);

        // Independent expected DC: round((200−128)·8 / 16) = round(576/16) = 36.
        let expected_dc = round_div_nearest((200 - 128) * 8, i32::from(quant[0]));
        assert_eq!(expected_dc, 36);
        assert_eq!(blocks[0].0, 36, "first block DC diff = quantized DC");
        for b in &blocks[1..] {
            assert_eq!(
                b.0, 0,
                "subsequent identical blocks predict to zero DC diff"
            );
        }
        for (_, coeffs) in &blocks {
            assert!(
                coeffs[1..].iter().all(|&c| c == 0),
                "constant block has no AC"
            );
        }
    }

    #[test]
    fn single_horizontal_frequency_lights_one_ac_coefficient() {
        // A pure horizontal cosine at the lowest AC frequency puts energy only in coefficient u=1,
        // v=0 (natural index 1). Decoding the block must show exactly that coefficient nonzero (plus
        // the DC term), pinning the zig-zag mapping and the run/size AC path.
        let mut data = vec![0u8; 8 * 8];
        for y in 0..8 {
            for x in 0..8 {
                // 128 + 100·cos((2x+1)π/16): a single-frequency horizontal wave, constant per column.
                let v = 128.0 + 100.0 * (((2 * x + 1) as f64) * std::f64::consts::PI / 16.0).cos();
                data[y * 8 + x] = v.round().clamp(0.0, 255.0) as u8;
            }
        }
        let plane = Plane {
            data,
            width: 8,
            height: 8,
        };
        let quant = quant::scale(&quant::LUMINANCE, 50);
        let dc = EncTable::from_spec(&huffman::STD_LUMA_DC);
        let ac = EncTable::from_spec(&huffman::STD_LUMA_AC);
        let comp = Component {
            h: 1,
            v: 1,
            plane: &plane,
            quant: &quant,
            dc: &dc,
            ac: &ac,
        };

        let mut entropy = Vec::new();
        encode_scan(&[comp], 8, 8, 0, &mut entropy);
        let (_, coeffs) = decode_blocks(&entropy, &dc, &ac, 1)[0];

        assert_ne!(coeffs[1], 0, "the u=1 coefficient must be lit");
        for (i, &c) in coeffs.iter().enumerate() {
            if i != 0 && i != 1 {
                assert_eq!(c, 0, "unexpected energy at natural index {i}");
            }
        }
    }

    // --- Direct §F.1.2 entropy-coder tests: feed `encode_quantized_block` hand-built coefficient
    // arrays and assert the exact emitted bytes against a hand-listed (code, length) sequence from
    // the standard tables (K.3/K.5 anchors are pinned in `huffman`'s own tests). ---

    /// Entropy-codes one hand-built block with the standard luma tables, flushing at the end.
    fn encode_one(q: &[i32; 64], dc_pred: &mut i32) -> Vec<u8> {
        let dc = EncTable::from_spec(&huffman::STD_LUMA_DC);
        let ac = EncTable::from_spec(&huffman::STD_LUMA_AC);
        let mut out = Vec::new();
        let mut w = BitWriter::new(&mut out);
        encode_quantized_block(q, dc_pred, &dc, &ac, &mut w);
        w.flush();
        out
    }

    /// Packs a hand-listed `(bits, length)` sequence with 1-padding — the expected-stream builder.
    fn expect_bits(seq: &[(u16, u8)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut w = BitWriter::new(&mut out);
        for &(v, n) in seq {
            w.write_bits(v, n);
        }
        w.flush();
        out
    }

    /// A block whose only nonzero AC sits after exactly `zeros` zig-zag zeros, with value 1.
    fn block_with_zero_run(zeros: usize) -> [i32; 64] {
        let mut q = [0i32; 64];
        q[ZIGZAG[zeros + 1]] = 1;
        q
    }

    // Standard-table codes used below (see huffman.rs tests for their Annex C derivations):
    //   luma DC cat 0 = 00₂;  luma AC ZRL (F/0) = 11111111001₂ (11 bits);
    //   0/1 = 00₂;  1/1 = 1100₂;  4/1 = 111011₂ (6 bits);  0/2 = 01₂;  EOB = 1010₂.
    const DC0: (u16, u8) = (0b00, 2);
    const ZRL: (u16, u8) = (0b111_1111_1001, 11);
    const EOB: (u16, u8) = (0b1010, 4);

    #[test]
    fn zero_run_of_exactly_16_is_one_zrl() {
        // 16 zeros then +1: ZRL, then 0/1 (run 0 after the ZRL) with one magnitude bit, then EOB.
        let got = encode_one(&block_with_zero_run(16), &mut 0);
        assert_eq!(got, expect_bits(&[DC0, ZRL, (0b00, 2), (1, 1), EOB]));
        // Literal anchor, fully hand-packed: 00|11111111001|00|1|1010|1111 → 3F C9 AF.
        assert_eq!(got, vec![0x3F, 0xC9, 0xAF]);
    }

    #[test]
    fn zero_runs_of_17_and_20_leave_a_remainder_run() {
        // 17 zeros: ZRL eats 16, the remaining run of 1 joins the symbol → 1/1 = 1100₂.
        assert_eq!(
            encode_one(&block_with_zero_run(17), &mut 0),
            expect_bits(&[DC0, ZRL, (0b1100, 4), (1, 1), EOB])
        );
        // 20 zeros: ZRL then run 4 → 4/1 = 111011₂. Distinguishes `run -= 16` from `run /= 16`
        // (both give run 1 at 17 zeros — 20 zeros is the case where they diverge: 4 vs 1).
        assert_eq!(
            encode_one(&block_with_zero_run(20), &mut 0),
            expect_bits(&[DC0, ZRL, (0b111011, 6), (1, 1), EOB])
        );
    }

    #[test]
    fn zero_run_of_33_is_two_zrls() {
        // 33 zeros: two ZRLs (32 zeros) then run 1 → 1/1. A `run /= 16` mutant emits only one ZRL.
        assert_eq!(
            encode_one(&block_with_zero_run(33), &mut 0),
            expect_bits(&[DC0, ZRL, ZRL, (0b1100, 4), (1, 1), EOB])
        );
    }

    #[test]
    fn trailing_zeros_end_in_eob() {
        // Natural index 1 (zig-zag position 1) holds +3 (category 2 → 0/2 = 01₂, bits 11₂); the 62
        // trailing zeros collapse into a single EOB.
        let mut q = [0i32; 64];
        q[1] = 3;
        assert_eq!(
            encode_one(&q, &mut 0),
            expect_bits(&[DC0, (0b01, 2), (0b11, 2), EOB])
        );
    }

    #[test]
    fn nonzero_last_coefficient_suppresses_eob() {
        // Every AC coefficient +1: 63 consecutive 0/1 symbols and NO EOB (§F.1.2.2 — EOB is sent
        // only when the block ends in zeros). A `run > 0` → `run >= 0` mutant appends a spurious
        // EOB, changing the bytes.
        let mut q = [1i32; 64];
        q[0] = 0;
        let mut seq = vec![DC0];
        for _ in 0..63 {
            seq.push((0b00, 2)); // 0/1
            seq.push((1, 1)); // magnitude +1
        }
        assert_eq!(encode_one(&q, &mut 0), expect_bits(&seq));
    }

    #[test]
    fn dc_differences_across_blocks() {
        // Three blocks with absolute DCs 5, 2, 2 sharing one predictor: diffs +5 (cat 3, DC code
        // 100₂, bits 101₂), −3 (cat 2, DC code 011₂, bits (−3−1)&11₂ = 00₂), 0 (cat 0, no bits).
        let mut out = Vec::new();
        let dc = EncTable::from_spec(&huffman::STD_LUMA_DC);
        let ac = EncTable::from_spec(&huffman::STD_LUMA_AC);
        let mut w = BitWriter::new(&mut out);
        let mut pred = 0i32;
        for dc_value in [5, 2, 2] {
            let mut q = [0i32; 64];
            q[0] = dc_value;
            encode_quantized_block(&q, &mut pred, &dc, &ac, &mut w);
        }
        w.flush();
        assert_eq!(pred, 2, "predictor tracks the last absolute DC");
        assert_eq!(
            out,
            expect_bits(&[
                (0b100, 3),
                (0b101, 3),
                EOB, // block 1: cat 3, +5
                (0b011, 3),
                (0b00, 2),
                EOB,       // block 2: cat 2, −3
                (0b00, 2), // block 3: cat 0, no magnitude bits
                EOB,
            ])
        );
    }

    // --- Reference-pipeline tests: encode per-pixel-distinct images, decode the scan with the
    // test decoder, and compare every block against an independently computed expectation
    // (gather with edge replication → fdct8x8 → round_div_nearest quantize). Distinct content is
    // the point: any mutated pixel/block/MCU coordinate reads different samples somewhere and
    // diverges. Solid colors could not see those mutants. ---

    /// The test-side reference for one 8×8 block of `plane` at block coords `(bx, by)`: the same
    /// §A.3.1/§A.3.3/§A.3.4 stages, written independently of the production gather.
    fn reference_block(plane: &Plane, bx: usize, by: usize, quant: &[u8; 64]) -> [i32; 64] {
        let mut block = [0i32; 64];
        for (i, cell) in block.iter_mut().enumerate() {
            let x = (bx * 8 + i % 8).min(plane.width - 1);
            let y = (by * 8 + i / 8).min(plane.height - 1);
            *cell = i32::from(plane.data[y * plane.width + x]) - 128;
        }
        fdct8x8(&mut block);
        let mut q = [0i32; 64];
        for (dst, (&coeff, &step)) in q.iter_mut().zip(block.iter().zip(quant.iter())) {
            *dst = round_div_nearest(coeff, i32::from(step));
        }
        q
    }

    /// Decodes an interleaved scan (no restart markers): one `(h, v, dc, ac)` per component.
    /// Returns, per component, the quantized blocks in emission order with the DC prediction
    /// resolved to absolute values.
    fn decode_interleaved(
        entropy: &[u8],
        comps: &[(u8, u8, &EncTable, &EncTable)],
        mcu_count: u32,
    ) -> Vec<Vec<[i32; 64]>> {
        let tables: Vec<_> = comps
            .iter()
            .map(|(_, _, dc, ac)| (invert(dc), invert(ac)))
            .collect();
        let mut reader = BitReader {
            bytes: entropy,
            pos: 0,
            bit: 0,
        };
        let mut preds = vec![0i32; comps.len()];
        let mut out = vec![Vec::new(); comps.len()];
        for _ in 0..mcu_count {
            for (ci, &(h, v, _, _)) in comps.iter().enumerate() {
                for _ in 0..usize::from(h) * usize::from(v) {
                    let mut coeffs = [0i32; 64];
                    let dc_cat = reader.decode_symbol(&tables[ci].0);
                    preds[ci] += reader.decode_value(dc_cat);
                    coeffs[0] = preds[ci];
                    let mut k = 1usize;
                    while k < 64 {
                        let rs = reader.decode_symbol(&tables[ci].1);
                        let (run, size) = (rs >> 4, rs & 0x0F);
                        if size == 0 {
                            if run == 15 {
                                k += 16; // ZRL
                                continue;
                            }
                            break; // EOB
                        }
                        k += run as usize;
                        coeffs[ZIGZAG[k]] = reader.decode_value(size);
                        k += 1;
                    }
                    out[ci].push(coeffs);
                }
            }
        }
        out
    }

    /// A deterministic per-pixel-distinct byte pattern (no two neighbours equal, varies on both
    /// axes) so every source coordinate is load-bearing.
    fn pattern(i: usize) -> u8 {
        ((i * 31 + 17) % 251) as u8
    }

    #[test]
    fn grayscale_blocks_match_reference_pipeline() {
        // 16×16 per-pixel-distinct grayscale = 2×2 blocks, emitted row-major. Every decoded block
        // must equal the independent reference at its (bx, by) — any mutated sample/block
        // coordinate in the production gather reads different pixels and diverges.
        let plane = Plane {
            data: (0..16 * 16).map(pattern).collect(),
            width: 16,
            height: 16,
        };
        let quant = quant::scale(&quant::LUMINANCE, 50);
        let dc = EncTable::from_spec(&huffman::STD_LUMA_DC);
        let ac = EncTable::from_spec(&huffman::STD_LUMA_AC);
        let comp = Component {
            h: 1,
            v: 1,
            plane: &plane,
            quant: &quant,
            dc: &dc,
            ac: &ac,
        };
        let mut entropy = Vec::new();
        encode_scan(&[comp], 16, 16, 0, &mut entropy);

        let decoded = decode_interleaved(&entropy, &[(1, 1, &dc, &ac)], 4);
        let mut n = 0;
        for by in 0..2 {
            for bx in 0..2 {
                assert_eq!(
                    decoded[0][n],
                    reference_block(&plane, bx, by, &quant),
                    "block ({bx},{by})"
                );
                n += 1;
            }
        }
    }

    #[test]
    fn color_444_blocks_match_reference_pipeline() {
        // 8×8 per-pixel-distinct RGB at 4:4:4: one MCU with block order Y, Cb, Cr. The reference
        // converts each pixel with the same T.871 §7 conversion in an independent loop, so any
        // mutation of the production `i*3(+1/+2)` channel indexing or the `w*h` conversion loop
        // bound produces different planes and diverges.
        let (w, h) = (8usize, 8usize);
        let rgb: Vec<u8> = (0..w * h * 3).map(pattern).collect();
        let img = ImageRef::<Rgb8>::new(&rgb, Dimensions::new(8, 8).unwrap()).unwrap();
        let jpeg = JpegEncoder::new()
            .with_quality(50)
            .with_subsampling(ChromaSubsampling::Ycbcr444)
            .encode_to_vec(img)
            .unwrap();
        let entropy = entropy_of(&jpeg);

        // Independent plane conversion.
        let mut planes = [
            Plane {
                data: vec![0; w * h],
                width: w,
                height: h,
            },
            Plane {
                data: vec![0; w * h],
                width: w,
                height: h,
            },
            Plane {
                data: vec![0; w * h],
                width: w,
                height: h,
            },
        ];
        for py in 0..h {
            for px in 0..w {
                let i = py * w + px;
                let (y, cb, cr) =
                    rgb_to_ycbcr(rgb[3 * i], rgb[3 * i + 1], rgb[3 * i + 2], ColorRange::Full);
                planes[0].data[i] = y;
                planes[1].data[i] = cb;
                planes[2].data[i] = cr;
            }
        }

        let lq = quant::scale(&quant::LUMINANCE, 50);
        let cq = quant::scale(&quant::CHROMINANCE, 50);
        let ldc = EncTable::from_spec(&huffman::STD_LUMA_DC);
        let lac = EncTable::from_spec(&huffman::STD_LUMA_AC);
        let cdc = EncTable::from_spec(&huffman::STD_CHROMA_DC);
        let cac = EncTable::from_spec(&huffman::STD_CHROMA_AC);
        let decoded = decode_interleaved(
            &entropy,
            &[(1, 1, &ldc, &lac), (1, 1, &cdc, &cac), (1, 1, &cdc, &cac)],
            1,
        );
        for (ci, quant) in [(0usize, &lq), (1, &cq), (2, &cq)] {
            assert_eq!(
                decoded[ci][0],
                reference_block(&planes[ci], 0, 0, quant),
                "component {ci}"
            );
        }
    }

    #[test]
    fn color_420_multi_mcu_matches_reference_pipeline() {
        // 32×32 per-pixel-distinct RGB at 4:2:0: 2×2 MCUs of 16×16, each carrying four luma blocks
        // (bx, by) ∈ 2×2 at plane coords (mx·2+bx, my·2+by) plus one block per chroma plane at
        // (mx, my). Comparing every block in emission order against the reference pins the MCU
        // block-coordinate arithmetic on both axes for h = v = 2 — including the vertical terms,
        // which the single-MCU-row cases can never distinguish (my = 0 masks `my*v` mutations).
        let (w, h) = (32usize, 32usize);
        let rgb: Vec<u8> = (0..w * h * 3).map(pattern).collect();
        let img = ImageRef::<Rgb8>::new(&rgb, Dimensions::new(32, 32).unwrap()).unwrap();
        let jpeg = JpegEncoder::new()
            .with_quality(50)
            .with_subsampling(ChromaSubsampling::Ycbcr420)
            .encode_to_vec(img)
            .unwrap();
        let entropy = entropy_of(&jpeg);

        // Independent plane conversion; chroma is then box-downsampled 2×2 (the pinned-elsewhere
        // production `downsample` is reused so this test focuses on the scan geometry).
        let mut y = vec![0u8; w * h];
        let mut cb = vec![0u8; w * h];
        let mut cr = vec![0u8; w * h];
        for i in 0..w * h {
            let (yy, u, v) =
                rgb_to_ycbcr(rgb[3 * i], rgb[3 * i + 1], rgb[3 * i + 2], ColorRange::Full);
            y[i] = yy;
            cb[i] = u;
            cr[i] = v;
        }
        let y_plane = Plane {
            data: y,
            width: w,
            height: h,
        };
        let cb_plane = downsample(&cb, w, h, 2, 2);
        let cr_plane = downsample(&cr, w, h, 2, 2);

        let lq = quant::scale(&quant::LUMINANCE, 50);
        let cq = quant::scale(&quant::CHROMINANCE, 50);
        let ldc = EncTable::from_spec(&huffman::STD_LUMA_DC);
        let lac = EncTable::from_spec(&huffman::STD_LUMA_AC);
        let cdc = EncTable::from_spec(&huffman::STD_CHROMA_DC);
        let cac = EncTable::from_spec(&huffman::STD_CHROMA_AC);
        let decoded = decode_interleaved(
            &entropy,
            &[(2, 2, &ldc, &lac), (1, 1, &cdc, &cac), (1, 1, &cdc, &cac)],
            4,
        );

        let mut luma_n = 0;
        let mut chroma_n = 0;
        for my in 0..2 {
            for mx in 0..2 {
                for by in 0..2 {
                    for bx in 0..2 {
                        assert_eq!(
                            decoded[0][luma_n],
                            reference_block(&y_plane, mx * 2 + bx, my * 2 + by, &lq),
                            "luma MCU ({mx},{my}) block ({bx},{by})"
                        );
                        luma_n += 1;
                    }
                }
                assert_eq!(
                    decoded[1][chroma_n],
                    reference_block(&cb_plane, mx, my, &cq),
                    "Cb MCU ({mx},{my})"
                );
                assert_eq!(
                    decoded[2][chroma_n],
                    reference_block(&cr_plane, mx, my, &cq),
                    "Cr MCU ({mx},{my})"
                );
                chroma_n += 1;
            }
        }
    }

    /// Extracts the entropy-coded bytes of a full JPEG stream: everything between the SOS segment
    /// and the trailing EOI, returned raw (still stuffed — the test decoder de-stuffs itself).
    fn entropy_of(jpeg: &[u8]) -> Vec<u8> {
        // Walk the header segments to find the end of SOS.
        let mut pos = 2; // past SOI
        loop {
            assert_eq!(jpeg[pos], 0xFF);
            let code = jpeg[pos + 1];
            let len = usize::from(u16::from_be_bytes([jpeg[pos + 2], jpeg[pos + 3]]));
            pos += 2 + len;
            if code == marker::code::SOS {
                break;
            }
        }
        jpeg[pos..jpeg.len() - 2].to_vec()
    }
}
