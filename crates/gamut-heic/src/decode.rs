//! The pluggable HEVC-intra decode pipeline (issue #238, slice S3): the [`HevcDecoder`] hook, the
//! decoder's [`DecodedFrame`] output contract, and the derivation/colour/transform pipeline that
//! turns a HEIF item into planar samples or a presentation-ready [`ImageBuf<Rgba8>`].
//!
//! # Where the codec boundary sits
//!
//! `gamut-heic` owns the *container* and everything around the coded picture — item derivation
//! (`grid`/`iden`/`iovl`), colour conversion, chroma upsampling, alpha merge, and the
//! `clap`/`irot`/`imir` transforms — but not the HEVC codestream itself (its RBSP/slice/CTU decode
//! is codec scope, issue #18, and on many platforms is better served by a hardware/OS decoder).
//! [`HevcDecoder`] is that seam: a caller implements HEVC-intra decode once, and every compliant
//! HEIF image — single items, grids, overlays, identity derivations, alpha, and the transformative
//! properties — decodes through this crate.
//!
//! # FFI-adaptable by construction
//!
//! [`HevcDecoder`] is deliberately object-safe and C-shaped so a `-sys` shim can wrap a platform
//! decoder behind a function-pointer table (the actual `gamut-ffi` wiring is a later crate concern):
//!
//! - **Plain byte-slice input.** [`decode_intra`](HevcDecoder::decode_intra) receives the raw,
//!   length-prefixed item payload as `&[u8]`; an implementation bridges it to its decoder with the
//!   existing [`HevcConfig::annex_b`] / [`crate::iter_nal_units`] helpers. The *pipeline* (not the
//!   implementation) calls [`HevcConfig::validate_still_payload`] first, so a non-still payload never
//!   reaches the hook.
//! - **POD-ish output.** [`DecodedFrame`] is uniform `u16` sample planes plus a handful of scalars
//!   ([`ChromaFormat`] is `#[repr(u8)]`), which map onto a C struct without a translation layer.
//! - **`&mut self`.** A stateful platform decoder (reused contexts, GPU handles) fits behind
//!   `&mut self`; a pure function fits too.
//!
//! # Two presentation surfaces
//!
//! - [`HeifImage::decode_item_planar`] — the **raw** surface. It resolves an item through its
//!   derivation (coded → decoder; `iden` → source; `grid` → tile assembly in the plane domain) and
//!   returns the decoder's [`DecodedFrame`] untouched by colour or transforms. Everything a caller
//!   could want is here, at full bit depth and native chroma; the caller drives colour/transforms
//!   with the [`HeifItem`](crate::HeifItem) property accessors. `iovl` compositing is *not* on this
//!   surface (it is defined over RGBA) — it returns [`Error::Unsupported`], directing callers to the
//!   RGBA path.
//! - [`HeifImage::decode_item_rgba8`] / [`HeifImage::decode_primary_rgba8`] — the **presentation**
//!   surface. It planar-decodes (or `iovl`-composites), converts colour, merges the alpha auxiliary,
//!   and applies the item's transformative properties in `ipma` order. It intentionally supports only
//!   the common 8-bit still-image colour cases (see [`HeifImage::decode_item_rgba8`]); anything
//!   outside them is [`Error::Unsupported`] on *this* surface while the planar surface still delivers
//!   the samples.

use gamut_color::{ColorRange, ycbcr_to_rgb};
use gamut_core::{Dimensions, Error, ImageBuf, Result, Rgba8};
use gamut_isobmff::ColourInformation;

use crate::hvcc::{ChromaFormat, HevcConfig};
use crate::image::{CleanAperture, HeifImage, HeifItem, ItemKind, TransformativeProperty};

/// The maximum derivation-reference nesting depth. A derived item (`grid`/`iden`/`iovl`) may
/// reference other derived items; this bounds the recursion so a deep (or, together with the
/// per-branch cycle check, a self-referential) derivation graph errors instead of overflowing the
/// stack.
const MAX_DERIVATION_DEPTH: usize = 8;

/// A pluggable HEVC-intra still-picture decoder: the seam issue #238 exposes so a user can hook in a
/// platform HEVC decoder (libde265, VideoToolbox, MediaCodec, a hardware block …) and decode
/// compliant HEIF images end to end through this crate.
///
/// Implement [`decode_intra`](Self::decode_intra) once; the crate's pipeline
/// ([`HeifImage::decode_item_planar`] / [`HeifImage::decode_item_rgba8`]) then handles item
/// derivation, colour, alpha, and the transformative properties around it. The trait is object-safe
/// (`&mut dyn HevcDecoder`) and byte-slice-shaped so it is straightforward to adapt across an FFI
/// boundary — see the crate-level and module documentation.
pub trait HevcDecoder {
    /// Decodes one HEVC-intra coded picture to a planar [`DecodedFrame`].
    ///
    /// `config` is the typed `hvcC` record of the item (parameter sets, chroma format, bit depth);
    /// `payload` is the raw, length-prefixed `hvc1`/`hev1` item payload. An implementation typically
    /// bridges the two into an Annex-B stream with [`HevcConfig::annex_b`] (which prepends `config`'s
    /// parameter sets) and feeds that to its decoder, then returns the reconstructed samples as a
    /// [`DecodedFrame`] whose `width`/`height` are the **post-conformance-window-crop** luma
    /// dimensions.
    ///
    /// The pipeline guarantees `payload` has already passed
    /// [`HevcConfig::validate_still_payload`] (every VCL NAL is an IRAP picture), so an
    /// implementation need not re-check the still-image constraint.
    ///
    /// # Errors
    ///
    /// Returns a [`gamut_core::Error`] if the codestream cannot be decoded — malformed bitstream,
    /// an unsupported HEVC tool or profile, and so on. The pipeline propagates it unchanged.
    fn decode_intra(&mut self, config: &HevcConfig, payload: &[u8]) -> Result<DecodedFrame>;

    /// Reports whether this backend can decode a picture with this `hvcC` configuration — the
    /// capability probe [`HevcDecoders`](crate::HevcDecoders) selects on.
    ///
    /// Returning `false` is the **only** signal that lets the registry fall through to the next
    /// backend (a backend that accepts and then fails propagates its error instead; see
    /// [`crate::BACKEND_DECLINED`] for the narrow late-decline escape hatch). Implementations
    /// typically check profile/tier/level, chroma format, and bit depth.
    ///
    /// Defaults to `true` — "I will attempt anything" — so a single plugged-in decoder needs no
    /// extra code and every pre-registry implementation keeps compiling unchanged.
    fn supports(&mut self, config: &HevcConfig) -> bool {
        let _ = config;
        true
    }
}

/// The owned planar output of a [`HevcDecoder`]: a YCbCr (or monochrome) frame at native chroma and
/// bit depth.
///
/// Samples are carried uniformly as `u16` whatever the `bit_depth` (an 8-bit frame simply uses the
/// low byte), so one plane layout serves 8..=16-bit content and maps cleanly onto a C buffer. The
/// planes are row-major: `y` is `width × height`; `cb`/`cr` are the chroma-plane dimensions of the
/// [`chroma`](Self::chroma) format (see [`ChromaFormat::chroma_dimensions`]) and are **empty** for
/// [`ChromaFormat::Monochrome`].
///
/// Construct with [`DecodedFrame::new`], which validates every plane length against
/// `width`/`height`/`chroma` (chroma dimensions use ceiling division, so an odd luma dimension is
/// handled) and the `bit_depth` range — an internally inconsistent frame is unrepresentable, so the
/// pipeline downstream can trust the plane sizes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedFrame {
    width: u32,
    height: u32,
    bit_depth: u8,
    chroma: ChromaFormat,
    y: Vec<u16>,
    cb: Vec<u16>,
    cr: Vec<u16>,
}

impl DecodedFrame {
    /// Builds a [`DecodedFrame`] from its planes, validating internal consistency.
    ///
    /// `width`/`height` are the luma dimensions (post conformance-window crop). The chroma planes
    /// `cb`/`cr` must match [`ChromaFormat::chroma_dimensions`] for `(width, height)` — which uses
    /// **ceiling** division on the subsampled axes, so a 5×3 luma frame in 4:2:0 has 3×2 chroma
    /// planes — and must both be **empty** for [`ChromaFormat::Monochrome`]. `bit_depth` must be in
    /// `8..=16`.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if either dimension is zero, if `bit_depth` is outside
    /// `8..=16`, or if any plane length does not match the dimensions for `chroma`.
    pub fn new(
        width: u32,
        height: u32,
        bit_depth: u8,
        chroma: ChromaFormat,
        y: Vec<u16>,
        cb: Vec<u16>,
        cr: Vec<u16>,
    ) -> Result<Self> {
        let luma = Dimensions::new(width, height)?
            .num_pixels()
            .ok_or(Error::InvalidInput("HEIF: frame dimensions overflow usize"))?;
        if !(8..=16).contains(&bit_depth) {
            return Err(Error::InvalidInput(
                "HEIF: frame bit depth out of range (8..=16)",
            ));
        }
        if y.len() != luma {
            return Err(Error::InvalidInput(
                "HEIF: luma plane length does not match dimensions",
            ));
        }
        let (cw, ch) = chroma.chroma_dimensions(width, height);
        // Cannot overflow: each chroma dimension is at most its luma dimension, and `luma` fit usize.
        let chroma_len = cw as usize * ch as usize;
        if chroma == ChromaFormat::Monochrome {
            if !cb.is_empty() || !cr.is_empty() {
                return Err(Error::InvalidInput(
                    "HEIF: monochrome frame must have empty chroma planes",
                ));
            }
        } else if cb.len() != chroma_len || cr.len() != chroma_len {
            return Err(Error::InvalidInput(
                "HEIF: chroma plane length does not match dimensions",
            ));
        }
        Ok(Self {
            width,
            height,
            bit_depth,
            chroma,
            y,
            cb,
            cr,
        })
    }

    /// The luma width in pixels (post conformance-window crop).
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The luma height in pixels (post conformance-window crop).
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The luma dimensions.
    #[must_use]
    pub fn dimensions(&self) -> Dimensions {
        Dimensions {
            width: self.width,
            height: self.height,
        }
    }

    /// The per-sample bit depth (`8..=16`). Samples are stored in `u16` regardless.
    #[must_use]
    pub fn bit_depth(&self) -> u8 {
        self.bit_depth
    }

    /// The chroma sampling format.
    #[must_use]
    pub fn chroma(&self) -> ChromaFormat {
        self.chroma
    }

    /// The chroma-plane dimensions `(width, height)` — `(0, 0)` for [`ChromaFormat::Monochrome`].
    #[must_use]
    pub fn chroma_dimensions(&self) -> (u32, u32) {
        self.chroma.chroma_dimensions(self.width, self.height)
    }

    /// The row-major luma plane (`width × height` samples).
    #[must_use]
    pub fn y(&self) -> &[u16] {
        &self.y
    }

    /// The row-major Cb plane (empty for [`ChromaFormat::Monochrome`]).
    #[must_use]
    pub fn cb(&self) -> &[u16] {
        &self.cb
    }

    /// The row-major Cr plane (empty for [`ChromaFormat::Monochrome`]).
    #[must_use]
    pub fn cr(&self) -> &[u16] {
        &self.cr
    }
}

impl HeifImage {
    /// Decodes an item to a raw planar [`DecodedFrame`] — the surface that delivers everything, with
    /// no colour conversion or transformative properties applied (drive those from the item's
    /// [`HeifItem`](crate::HeifItem) accessors).
    ///
    /// The item is resolved through its derivation, recursively and depth-limited (a self- or
    /// mutually-referential `dimg` graph errors rather than recursing forever):
    ///
    /// - **Coded** (`hvc1`/`hev1`): the payload is validated against the still-image IRAP constraint
    ///   ([`HevcConfig::validate_still_payload`]) and handed to `decoder`.
    /// - **`iden`**: the frame of its single `dimg` source.
    /// - **`grid`**: every tile is decoded (row-major `dimg` order) and assembled in the plane
    ///   domain onto a `columns·tile_w × rows·tile_h` canvas, then cropped to the grid's output size
    ///   (ISO/IEC 23008-12 §6.6.2.3.2). All tiles must share chroma format, bit depth, and
    ///   dimensions, else [`Error::Unsupported`]; the arithmetic is checked so a hostile grid cannot
    ///   amplify allocation.
    /// - **`iovl`**: [`Error::Unsupported`] — overlay compositing is defined over RGBA; use
    ///   [`decode_item_rgba8`](Self::decode_item_rgba8).
    ///
    /// Transformative properties (`clap`/`irot`/`imir`) are **not** applied here — read them via
    /// [`HeifItem::transformative_properties`](crate::HeifItem::transformative_properties).
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] for a missing item, a malformed `hvcC`, a non-still payload, a
    /// derivation cycle, or a malformed derivation; [`Error::Unsupported`] for an `iovl`, a non-HEVC
    /// coded item, an item with an unrecognised essential property, a non-uniform grid, or a
    /// derivation nested past the internal depth limit; and propagates the `decoder`'s errors.
    pub fn decode_item_planar(
        &self,
        id: u32,
        decoder: &mut dyn HevcDecoder,
    ) -> Result<DecodedFrame> {
        let mut stack = Vec::new();
        self.decode_planar_inner(id, decoder, &mut stack)
    }

    /// Decodes an item to a presentation-ready [`ImageBuf<Rgba8>`]: planar decode (or `iovl`
    /// compositing), colour conversion, alpha-auxiliary merge, then the item's transformative
    /// properties applied in `ipma` order.
    ///
    /// **Colour policy (8-bit only).** This convenience surface supports the common still-image
    /// cases; anything else is [`Error::Unsupported`] *here* while
    /// [`decode_item_planar`](Self::decode_item_planar) still delivers the samples:
    ///
    /// - **nclx matrix 6 / 5 (BT.601)** — via [`gamut_color::ycbcr_to_rgb`], with the nclx
    ///   `full_range` flag selecting [`ColorRange`].
    /// - **nclx matrix 0 (identity / GBR)** — requires 4:4:4; samples are mapped directly
    ///   (`R=Cr, G=Y, B=Cb`) with no range expansion.
    /// - **Monochrome** — luma replicated to gray, range-expanded for limited range.
    /// - **Missing `colr`** — defaults to **BT.601, limited range** (justification below).
    /// - Anything else — BT.709/2020, an ICC-only `colr`, or a `bit_depth > 8` frame — is
    ///   [`Error::Unsupported`].
    ///
    /// The missing-`colr` default is BT.601 limited range: with no nclx present the matrix is
    /// undefined, and libheif assumes BT.601 for undefined/unspecified matrix coefficients; the range
    /// defaults to limited because the HEVC VUI `video_full_range_flag` defaults to 0 (ITU-T H.265
    /// §E.2.1) and still-image content is overwhelmingly studio-swing.
    ///
    /// **Chroma upsampling** is nearest-neighbour, co-sited (4:2:0 / 4:2:2 → 4:4:4) — the deliberate
    /// space/time tradeoff for a container-layer presentation helper; a caller wanting a better
    /// resampler works from the planar surface.
    ///
    /// **Alpha.** The item's alpha auxiliary (if any) is decoded through the same planar path, must
    /// be monochrome and match the master's **pre-transform** dimensions, and is scaled from its bit
    /// depth to 8-bit; an absent alpha is opaque (255). Premultiplication is **not** undone — query
    /// [`HeifImage::is_premultiplied`] and un-premultiply yourself if needed (an [`Rgba8`] buffer
    /// cannot carry the flag).
    ///
    /// **Transforms** are applied to the interleaved RGBA in `ipma` order (ISO/IEC 23008-12 §7): even
    /// when the order violates the MIAF constraint (check with
    /// [`HeifItem::is_miaf_transform_ordered`](crate::HeifItem::is_miaf_transform_ordered)) they are
    /// applied as listed. `clap` crops per ISO/IEC 14496-12 §12.1.4, `irot` rotates anti-clockwise,
    /// and `imir` exchanges top and bottom for axis 0 or left and right for axis 1.
    ///
    /// # Errors
    ///
    /// As [`decode_item_planar`](Self::decode_item_planar), plus [`Error::Unsupported`] for an
    /// unsupported colour configuration and [`Error::InvalidInput`] for a mismatched alpha auxiliary
    /// or an out-of-range/non-integer `clap`.
    pub fn decode_item_rgba8(
        &self,
        id: u32,
        decoder: &mut dyn HevcDecoder,
    ) -> Result<ImageBuf<Rgba8>> {
        let mut stack = Vec::new();
        self.decode_rgba_inner(id, decoder, &mut stack)
    }

    /// Decodes the primary item to a presentation-ready [`ImageBuf<Rgba8>`].
    ///
    /// Equivalent to [`decode_item_rgba8`](Self::decode_item_rgba8) on the primary item id.
    ///
    /// # Errors
    ///
    /// As [`decode_item_rgba8`](Self::decode_item_rgba8).
    pub fn decode_primary_rgba8(&self, decoder: &mut dyn HevcDecoder) -> Result<ImageBuf<Rgba8>> {
        self.decode_item_rgba8(self.as_isobmff().primary_item_id, decoder)
    }

    // ---- planar pipeline ---------------------------------------------------------------------

    fn decode_planar_inner(
        &self,
        id: u32,
        decoder: &mut dyn HevcDecoder,
        stack: &mut Vec<u32>,
    ) -> Result<DecodedFrame> {
        let item = self
            .item(id)
            .ok_or(Error::InvalidInput("HEIF: item id names no item"))?;
        if item.has_unsupported_essential_property() {
            return Err(Error::Unsupported(
                "HEIF: item has an unrecognised essential property",
            ));
        }
        enter_derivation(stack, id)?;
        let out = self.decode_planar_kind(id, item, decoder, stack);
        stack.pop();
        out
    }

    fn decode_planar_kind(
        &self,
        id: u32,
        item: HeifItem<'_>,
        decoder: &mut dyn HevcDecoder,
        stack: &mut Vec<u32>,
    ) -> Result<DecodedFrame> {
        match item.kind() {
            ItemKind::CodedImage { .. } if item.kind().is_hevc() => {
                let config = item.hevc_config().ok_or(Error::InvalidInput(
                    "HEIF: HEVC item has no hvcC configuration",
                ))??;
                let payload = &item.as_isobmff_item().payload;
                // The pipeline enforces the still-image constraint before the codec hook sees the
                // payload (a non-IRAP VCL NAL never reaches `decode_intra`).
                config.validate_still_payload(payload)?;
                decoder.decode_intra(&config, payload)
            }
            ItemKind::CodedImage { .. } => Err(Error::Unsupported(
                "HEIF: only HEVC coded items are decodable here",
            )),
            ItemKind::Identity => {
                let [source] = item.derivation_target_ids() else {
                    return Err(Error::InvalidInput(
                        "HEIF: iden item must reference exactly one source",
                    ));
                };
                self.decode_planar_inner(*source, decoder, stack)
            }
            ItemKind::Grid => self.decode_grid(id, item, decoder, stack),
            ItemKind::Overlay => Err(Error::Unsupported(
                "HEIF: overlay compositing is not available on the planar surface (use the RGBA path)",
            )),
            ItemKind::Exif | ItemKind::Mime | ItemKind::Unknown(_) => {
                Err(Error::InvalidInput("HEIF: item is not a decodable image"))
            }
        }
    }

    fn decode_grid(
        &self,
        id: u32,
        item: HeifItem<'_>,
        decoder: &mut dyn HevcDecoder,
        stack: &mut Vec<u32>,
    ) -> Result<DecodedFrame> {
        // `grid` validates the payload and that the tile count equals rows * columns.
        let grid = self.grid(id)?;
        let cols = usize::from(grid.columns);
        let rows = usize::from(grid.rows);
        let mut tiles = Vec::with_capacity(cols * rows);
        for &tile_id in item.derivation_target_ids() {
            tiles.push(self.decode_planar_inner(tile_id, decoder, stack)?);
        }

        // A grid always has >= 1 tile (rows, columns are >= 1). Every tile must be uniform.
        let first = &tiles[0];
        let (chroma, bit_depth, tw, th) =
            (first.chroma, first.bit_depth, first.width, first.height);
        if tiles.iter().any(|t| {
            t.chroma != chroma || t.bit_depth != bit_depth || t.width != tw || t.height != th
        }) {
            return Err(Error::Unsupported(
                "HEIF: grid tiles are not uniform in dimensions, chroma, or bit depth",
            ));
        }

        // Canvas = columns·tile_w × rows·tile_h, checked; the output must fit within it.
        let canvas_w = (cols as u32)
            .checked_mul(tw)
            .ok_or(Error::InvalidInput("HEIF: grid canvas width overflow"))?;
        let canvas_h = (rows as u32)
            .checked_mul(th)
            .ok_or(Error::InvalidInput("HEIF: grid canvas height overflow"))?;
        let (ow, oh) = (grid.output_width, grid.output_height);
        if ow == 0 || oh == 0 || ow > canvas_w || oh > canvas_h {
            return Err(Error::InvalidInput(
                "HEIF: grid output dimensions exceed the tiled canvas",
            ));
        }

        let y = assemble_plane(
            &tiles,
            DecodedFrame::y,
            cols,
            tw as usize,
            th as usize,
            ow as usize,
            oh as usize,
        );
        let (cb, cr) = if chroma == ChromaFormat::Monochrome {
            (Vec::new(), Vec::new())
        } else {
            let (tcw, tch) = chroma.chroma_dimensions(tw, th);
            let (ocw, och) = chroma.chroma_dimensions(ow, oh);
            let cb = assemble_plane(
                &tiles,
                DecodedFrame::cb,
                cols,
                tcw as usize,
                tch as usize,
                ocw as usize,
                och as usize,
            );
            let cr = assemble_plane(
                &tiles,
                DecodedFrame::cr,
                cols,
                tcw as usize,
                tch as usize,
                ocw as usize,
                och as usize,
            );
            (cb, cr)
        };
        DecodedFrame::new(ow, oh, bit_depth, chroma, y, cb, cr)
    }

    // ---- RGBA pipeline -----------------------------------------------------------------------

    fn decode_rgba_inner(
        &self,
        id: u32,
        decoder: &mut dyn HevcDecoder,
        stack: &mut Vec<u32>,
    ) -> Result<ImageBuf<Rgba8>> {
        let item = self
            .item(id)
            .ok_or(Error::InvalidInput("HEIF: item id names no item"))?;
        if item.has_unsupported_essential_property() {
            return Err(Error::Unsupported(
                "HEIF: item has an unrecognised essential property",
            ));
        }

        let (rgba, w, h) = if matches!(item.kind(), ItemKind::Overlay) {
            enter_derivation(stack, id)?;
            let composited = self.composite_overlay(id, item, decoder, stack);
            stack.pop();
            composited?
        } else {
            // The planar path applies its own cycle/depth guard for `id`.
            let frame = self.decode_planar_inner(id, decoder, stack)?;
            let mut rgba = frame_to_rgba(&item, &frame)?;
            let (w, h) = (frame.width, frame.height);
            if let Some(alpha) = self.alpha_auxiliary_of(id) {
                let alpha_frame = self.decode_planar_inner(alpha.id(), decoder, stack)?;
                apply_alpha(&mut rgba, w as usize, h as usize, &alpha_frame)?;
            }
            (rgba, w, h)
        };

        let (rgba, w, h) = apply_transforms(item.transformative_properties(), rgba, w, h)?;
        ImageBuf::<Rgba8>::new(rgba, Dimensions::new(w, h)?)
    }

    fn composite_overlay(
        &self,
        id: u32,
        item: HeifItem<'_>,
        decoder: &mut dyn HevcDecoder,
        stack: &mut Vec<u32>,
    ) -> Result<(Vec<u8>, u32, u32)> {
        // `overlay` validates the payload holds exactly one offset per `dimg` reference.
        let overlay = self.overlay(id)?;
        let (ow, oh) = (overlay.output_width, overlay.output_height);
        let len = (ow as usize)
            .checked_mul(oh as usize)
            .and_then(|p| p.checked_mul(4))
            .filter(|_| ow != 0 && oh != 0)
            .ok_or(Error::InvalidInput(
                "HEIF: overlay canvas is empty or too large",
            ))?;
        // Fill: 16-bit canvas samples scaled to 8-bit by >> 8.
        let fill = [
            (overlay.canvas_fill_value[0] >> 8) as u8,
            (overlay.canvas_fill_value[1] >> 8) as u8,
            (overlay.canvas_fill_value[2] >> 8) as u8,
            (overlay.canvas_fill_value[3] >> 8) as u8,
        ];
        let mut canvas = vec![0u8; len];
        for px in canvas.chunks_exact_mut(4) {
            px.copy_from_slice(&fill);
        }
        for (&input_id, &(dx, dy)) in item.derivation_target_ids().iter().zip(&overlay.offsets) {
            let src = self.decode_rgba_inner(input_id, decoder, stack)?;
            composite_over(
                &mut canvas,
                (ow, oh),
                src.as_samples(),
                (src.width(), src.height()),
                (dx, dy),
            );
        }
        Ok((canvas, ow, oh))
    }
}

// ---- derivation recursion guard --------------------------------------------------------------

/// Cycle- and depth-checks a derivation step, then records `id` on the recursion `stack`. The caller
/// pops `id` when the step completes.
fn enter_derivation(stack: &mut Vec<u32>, id: u32) -> Result<()> {
    if stack.contains(&id) {
        return Err(Error::InvalidInput("HEIF: derivation reference cycle"));
    }
    if stack.len() >= MAX_DERIVATION_DEPTH {
        return Err(Error::Unsupported("HEIF: derivation nested too deeply"));
    }
    stack.push(id);
    Ok(())
}

// ---- grid plane assembly ---------------------------------------------------------------------

/// Assembles one plane of a grid directly into its cropped output: for each output sample it reads
/// the covering tile's plane, so the tile canvas (`columns·tile_pw × rows·tile_ph`) is sampled and
/// cropped to `out_pw × out_ph` in a single pass. `out_pw <= columns·tile_pw` and
/// `out_ph <= rows·tile_ph` are guaranteed by the caller, so the tile index is always in range.
fn assemble_plane(
    tiles: &[DecodedFrame],
    plane: fn(&DecodedFrame) -> &[u16],
    cols: usize,
    tile_pw: usize,
    tile_ph: usize,
    out_pw: usize,
    out_ph: usize,
) -> Vec<u16> {
    let mut out = vec![0u16; out_pw * out_ph];
    for oy in 0..out_ph {
        let (tile_row, in_y) = (oy / tile_ph, oy % tile_ph);
        for ox in 0..out_pw {
            let (tile_col, in_x) = (ox / tile_pw, ox % tile_pw);
            let tile = &tiles[tile_row * cols + tile_col];
            out[oy * out_pw + ox] = plane(tile)[in_y * tile_pw + in_x];
        }
    }
    out
}

// ---- colour conversion -----------------------------------------------------------------------

/// The presentation matrix coefficients and signal range for an item: its nclx `colr`, or the
/// documented BT.601-limited default when absent.
fn colour_params(item: &HeifItem<'_>) -> Result<(u16, ColorRange)> {
    match item.colour() {
        // Default when no `colr`: BT.601 (matrix 6), limited range — see `decode_item_rgba8`.
        None => Ok((6, ColorRange::Limited)),
        Some(ColourInformation::Nclx(nclx)) => {
            let range = if nclx.full_range {
                ColorRange::Full
            } else {
                ColorRange::Limited
            };
            Ok((nclx.matrix_coefficients, range))
        }
        Some(_) => Err(Error::Unsupported(
            "HEIF: ICC-profile colr is not supported on the RGBA surface (use the planar surface)",
        )),
    }
}

/// Converts a planar [`DecodedFrame`] to an interleaved 8-bit RGBA buffer (opaque alpha), applying
/// this crate's colour policy (see [`HeifImage::decode_item_rgba8`]).
fn frame_to_rgba(item: &HeifItem<'_>, frame: &DecodedFrame) -> Result<Vec<u8>> {
    if frame.bit_depth != 8 {
        return Err(Error::Unsupported(
            "HEIF: >8-bit RGBA presentation is not yet supported (use the planar surface)",
        ));
    }
    let (matrix, range) = colour_params(item)?;
    let (w, h) = (frame.width as usize, frame.height as usize);
    let mut out = vec![0u8; w * h * 4];

    if frame.chroma == ChromaFormat::Monochrome {
        for (i, px) in out.chunks_exact_mut(4).enumerate() {
            let g = expand_gray8(frame.y[i], range);
            px.copy_from_slice(&[g, g, g, 255]);
        }
        return Ok(out);
    }

    match matrix {
        // Identity / GBR (mc = 0): plane order Y=G, Cb=B, Cr=R; requires 4:4:4, no range expansion.
        0 => {
            if frame.chroma != ChromaFormat::Yuv444 {
                return Err(Error::Unsupported(
                    "HEIF: identity matrix (mc = 0) requires 4:4:4 chroma",
                ));
            }
            for (i, px) in out.chunks_exact_mut(4).enumerate() {
                px.copy_from_slice(&[frame.cr[i] as u8, frame.y[i] as u8, frame.cb[i] as u8, 255]);
            }
        }
        // BT.601 (matrix 6) and BT.470 System B,G (matrix 5) share coefficients.
        5 | 6 => {
            let (cw, _) = frame.chroma.chroma_dimensions(frame.width, frame.height);
            let cw = cw as usize;
            for y in 0..h {
                let cy = match frame.chroma {
                    ChromaFormat::Yuv420 => y / 2,
                    _ => y,
                };
                for x in 0..w {
                    let cx = match frame.chroma {
                        ChromaFormat::Yuv444 => x,
                        _ => x / 2,
                    };
                    let ci = cy * cw + cx;
                    let (r, g, b) = ycbcr_to_rgb(
                        frame.y[y * w + x] as u8,
                        frame.cb[ci] as u8,
                        frame.cr[ci] as u8,
                        range,
                    );
                    let o = (y * w + x) * 4;
                    out[o..o + 4].copy_from_slice(&[r, g, b, 255]);
                }
            }
        }
        _ => {
            return Err(Error::Unsupported(
                "HEIF: only BT.601 (matrix 5/6), identity (0), and monochrome are supported on the RGBA surface",
            ));
        }
    }
    Ok(out)
}

/// Expands one 8-bit monochrome luma sample to a display gray value: identity for full range, and
/// the studio-swing expansion `(y - 16) · 255 / 219` (rounded, clamped) for limited range.
fn expand_gray8(y: u16, range: ColorRange) -> u8 {
    match range {
        ColorRange::Full => y.min(255) as u8,
        ColorRange::Limited => {
            let expanded = ((i32::from(y) - 16) * 255 + 109) / 219;
            expanded.clamp(0, 255) as u8
        }
    }
}

/// Merges an alpha auxiliary into the RGBA buffer's alpha channel.
fn apply_alpha(rgba: &mut [u8], w: usize, h: usize, alpha: &DecodedFrame) -> Result<()> {
    if alpha.chroma != ChromaFormat::Monochrome {
        return Err(Error::InvalidInput(
            "HEIF: alpha auxiliary must be monochrome",
        ));
    }
    if alpha.width as usize != w || alpha.height as usize != h {
        return Err(Error::InvalidInput(
            "HEIF: alpha auxiliary dimensions do not match the master image",
        ));
    }
    let max = (1u32 << alpha.bit_depth) - 1;
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        // Scale the sample from its native bit depth to 8-bit, round-to-nearest. Depth 8 needs no
        // special case: `(s * 255 + 127) / 255 == s` for every `s` in `0..=255`, so this general
        // rescale is already the identity there.
        let s = u32::from(alpha.y[i]);
        px[3] = ((s * 255 + max / 2) / max) as u8;
    }
    Ok(())
}

// ---- transformative properties (on interleaved RGBA8) ----------------------------------------

/// Applies transformative properties to an interleaved RGBA buffer in `ipma` (list) order, returning
/// the transformed buffer and its dimensions.
fn apply_transforms(
    props: Vec<TransformativeProperty>,
    rgba: Vec<u8>,
    w: u32,
    h: u32,
) -> Result<(Vec<u8>, u32, u32)> {
    let (mut cur, mut cw, mut ch) = (rgba, w, h);
    for tp in props {
        match tp {
            TransformativeProperty::Rotation(turns) => {
                // `turns` is 0..=3 by construction — the isobmff `irot` parser masks the angle field
                // with `& 0x03` — so each unit is exactly one 90° anti-clockwise rotation and no
                // modulo normalisation is needed here.
                for _ in 0..turns {
                    let rotated = rotate90_ccw(&cur, cw, ch);
                    cur = rotated.0;
                    cw = rotated.1;
                    ch = rotated.2;
                }
            }
            TransformativeProperty::Mirror(axis) => {
                cur = mirror(&cur, cw, ch, axis);
            }
            TransformativeProperty::CleanAperture(clap) => {
                let cropped = crop_clap(&cur, cw, ch, &clap)?;
                cur = cropped.0;
                cw = cropped.1;
                ch = cropped.2;
            }
        }
    }
    Ok((cur, cw, ch))
}

/// Rotates an interleaved RGBA buffer 90° anti-clockwise: output `(ox, oy)` reads input
/// `(w - 1 - oy, ox)`, giving output dimensions `(h, w)`.
fn rotate90_ccw(src: &[u8], w: u32, h: u32) -> (Vec<u8>, u32, u32) {
    let (wu, hu) = (w as usize, h as usize);
    let (nw, nh) = (hu, wu);
    let mut out = vec![0u8; nw * nh * 4];
    for oy in 0..nh {
        for ox in 0..nw {
            let (sx, sy) = (wu - 1 - oy, ox);
            let si = (sy * wu + sx) * 4;
            let oi = (oy * nw + ox) * 4;
            out[oi..oi + 4].copy_from_slice(&src[si..si + 4]);
        }
    }
    (out, h, w)
}

/// Mirrors an interleaved RGBA buffer by exchanging top and bottom (`axis = 0`) or left and right
/// (`axis = 1`); dimensions are unchanged. Any other `axis` value is treated as axis 0 (the `imir`
/// field is a single bit).
fn mirror(src: &[u8], w: u32, h: u32, axis: u8) -> Vec<u8> {
    let (wu, hu) = (w as usize, h as usize);
    let mut out = vec![0u8; wu * hu * 4];
    for y in 0..hu {
        for x in 0..wu {
            let (sx, sy) = if axis == 1 {
                (wu - 1 - x, y)
            } else {
                (x, hu - 1 - y)
            };
            let si = (sy * wu + sx) * 4;
            let oi = (y * wu + x) * 4;
            out[oi..oi + 4].copy_from_slice(&src[si..si + 4]);
        }
    }
    out
}

/// Crops an interleaved RGBA buffer to its `clap` clean aperture (ISO/IEC 14496-12 §12.1.4).
///
/// The clean aperture is a `cleanApertureWidth × cleanApertureHeight` rectangle centred at
/// `((w - 1)/2 + horizOff, (h - 1)/2 + vertOff)`, so its top-left corner is
/// `left = (w - cleanApertureWidth)/2 + horizOff` and `top = (h - cleanApertureHeight)/2 + vertOff`
/// (the width/height and offset centre terms combine exactly, matching libavif's integer clap
/// conversion). The crop must be integer-valued and lie fully inside the image; otherwise it is
/// rejected.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if any denominator is zero, if the width/height or the computed
/// top-left corner is not integer-valued, or if the crop window falls outside the image.
fn crop_clap(src: &[u8], w: u32, h: u32, clap: &CleanAperture) -> Result<(Vec<u8>, u32, u32)> {
    if clap.width_d == 0 || clap.height_d == 0 || clap.horiz_off_d == 0 || clap.vert_off_d == 0 {
        return Err(Error::InvalidInput("HEIF: clap has a zero denominator"));
    }
    if !clap.width_n.is_multiple_of(clap.width_d) || !clap.height_n.is_multiple_of(clap.height_d) {
        return Err(Error::InvalidInput(
            "HEIF: clap width/height is not integer-valued",
        ));
    }
    let crop_w = clap.width_n / clap.width_d;
    let crop_h = clap.height_n / clap.height_d;
    if crop_w == 0 || crop_h == 0 {
        return Err(Error::InvalidInput("HEIF: clap has a zero-sized crop"));
    }
    let left = clap_offset(
        i64::from(w),
        i64::from(crop_w),
        clap.horiz_off_n,
        clap.horiz_off_d,
    )?;
    let top = clap_offset(
        i64::from(h),
        i64::from(crop_h),
        clap.vert_off_n,
        clap.vert_off_d,
    )?;
    if left < 0
        || top < 0
        || left + i64::from(crop_w) > i64::from(w)
        || top + i64::from(crop_h) > i64::from(h)
    {
        return Err(Error::InvalidInput(
            "HEIF: clap crop window lies outside the image",
        ));
    }

    let (wu, left, top) = (w as usize, left as usize, top as usize);
    let (cw, chh) = (crop_w as usize, crop_h as usize);
    let mut out = vec![0u8; cw * chh * 4];
    for y in 0..chh {
        for x in 0..cw {
            let si = ((top + y) * wu + (left + x)) * 4;
            let oi = (y * cw + x) * 4;
            out[oi..oi + 4].copy_from_slice(&src[si..si + 4]);
        }
    }
    Ok((out, crop_w, crop_h))
}

/// The integer top-left offset of a clean-aperture crop along one axis:
/// `left = (dim - crop)/2 + off_n/off_d`, evaluated exactly as `((dim - crop)·off_d + 2·off_n) /
/// (2·off_d)`. `off_n` is signed (two's-complement bits); `off_d` is a positive denominator.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if the offset is not integer-valued.
fn clap_offset(dim: i64, crop: i64, off_n: u32, off_d: u32) -> Result<i64> {
    let num = (dim - crop) * i64::from(off_d) + 2 * i64::from(off_n as i32);
    let den = 2 * i64::from(off_d);
    if num % den != 0 {
        return Err(Error::InvalidInput(
            "HEIF: clap offset is not integer-valued",
        ));
    }
    Ok(num / den)
}

// ---- overlay compositing ---------------------------------------------------------------------

/// Composites a source RGBA image over the canvas at signed offset `(dx, dy)`, clipping to the
/// canvas, with unassociated-alpha source-over blending.
///
/// Source-over in integer math (`sa`/`da` the source/destination alpha, `sc`/`dc` a channel):
/// `out_a = sa + round(da·(255 - sa)/255)`, and each channel
/// `out_c = round((sc·sa + round(dc·da·(255 - sa)/255)) / out_a)`, or a fully transparent pixel when
/// `out_a = 0`. Rounding is round-half-up (`+ half`).
fn composite_over(
    canvas: &mut [u8],
    canvas_dims: (u32, u32),
    src: &[u8],
    src_dims: (u32, u32),
    offset: (i32, i32),
) {
    let (cw, ch) = canvas_dims;
    let (sw, sh) = src_dims;
    let (dx, dy) = offset;
    for sy in 0..sh as i64 {
        let y = i64::from(dy) + sy;
        if y < 0 || y >= i64::from(ch) {
            continue;
        }
        for sx in 0..sw as i64 {
            let x = i64::from(dx) + sx;
            if x < 0 || x >= i64::from(cw) {
                continue;
            }
            let si = ((sy * i64::from(sw) + sx) * 4) as usize;
            let di = ((y * i64::from(cw) + x) * 4) as usize;
            let sa = u32::from(src[si + 3]);
            let inv = 255 - sa;
            let da = u32::from(canvas[di + 3]);
            let da_contrib = (da * inv + 127) / 255;
            let out_a = sa + da_contrib;
            if out_a == 0 {
                canvas[di..di + 4].copy_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            for c in 0..3 {
                let sc = u32::from(src[si + c]);
                let dc = u32::from(canvas[di + c]);
                let num = sc * sa + (dc * da * inv + 127) / 255;
                canvas[di + c] = ((num + out_a / 2) / out_a).min(255) as u8;
            }
            canvas[di + 3] = out_a as u8;
        }
    }
}
