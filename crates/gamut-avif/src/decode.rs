//! The pluggable AV1 still-picture decode pipeline (issue #250): the [`Av1StillDecoder`] hook, the
//! decoder's [`DecodedFrame`] output contract, and the derivation pipeline that turns an AVIF item
//! into planar samples or a presentation-ready [`ImageBuf<Rgba8>`](gamut_core::ImageBuf).
//!
//! # Where the codec boundary sits
//!
//! `gamut-avif` owns the *container* and everything around the coded picture — item derivation
//! (`grid`/`iden`/`iovl`), colour conversion, chroma upsampling, alpha merge, and the
//! `clap`/`irot`/`imir` transforms — but not the AV1 codestream itself (OBU payload decoding is
//! codec scope, and on many platforms is better served by a hardware/OS decoder — exactly the
//! split downstream `rawshift` drives with VideoToolbox/VAAPI/MediaCodec). [`Av1StillDecoder`] is
//! that seam: a caller implements AV1 still decode once, and every compliant AVIF image — single
//! items, grids, overlays, identity derivations, alpha, and the transformative properties —
//! decodes through this crate.
//!
//! The trait name fulfils the `Av1StillDecoder` reservation the workspace codec-seam plan
//! (issue #274) recorded for this crate's decode surface; the backend registry and
//! `gamut-codec-abi` adapter around it are the deferred follow-up (issue #241's program) and land
//! additively.
//!
//! # FFI-adaptable by construction
//!
//! [`Av1StillDecoder`] is deliberately object-safe and C-shaped so a `-sys` shim can wrap a
//! platform decoder behind a function-pointer table:
//!
//! - **Plain byte-slice input.** [`decode_still`](Av1StillDecoder::decode_still) receives the raw
//!   item payload (a low-overhead OBU stream) as `&[u8]` together with the typed
//!   [`Av1Config`]; an implementation either hands the pair to a decoder that accepts
//!   `av1C` + sample data directly, or bridges them into one self-contained temporal unit with
//!   [`Av1Config::full_stream`]. The *pipeline* (not the implementation) calls
//!   [`Av1Config::validate_still_payload`] first, so a payload violating the still-image
//!   constraints never reaches the hook.
//! - **POD-ish output.** [`DecodedFrame`] is uniform `u16` sample planes plus a handful of
//!   scalars ([`ChromaFormat`] is `#[repr(u8)]`), which map onto a C struct without a translation
//!   layer.
//! - **`&mut self`.** A stateful platform decoder (reused contexts, GPU handles) fits behind
//!   `&mut self`; a pure function fits too.
//!
//! # The planar surface
//!
//! [`AvifImage::decode_item_planar`] is the **raw** surface. It resolves an item through its
//! derivation (coded → decoder; `iden` → source; `grid` → tile assembly in the plane domain) and
//! returns the decoder's [`DecodedFrame`] untouched by colour or transforms. Everything a caller
//! could want is here, at full bit depth and native chroma; the caller drives colour/transforms
//! with the [`AvifItem`](crate::AvifItem) property accessors. `iovl` compositing is *not* on this
//! surface (it is defined over RGBA) — it returns [`Error::Unsupported`].

use gamut_core::{Dimensions, Error, Result};

use crate::av1c::{Av1Config, ChromaFormat};
use crate::image::{AvifImage, AvifItem, ItemKind};

/// The maximum derivation-reference nesting depth. A derived item (`grid`/`iden`/`iovl`) may
/// reference other derived items; this bounds the recursion so a deep (or, together with the
/// per-branch cycle check, a self-referential) derivation graph errors instead of overflowing the
/// stack.
const MAX_DERIVATION_DEPTH: usize = 8;

/// A pluggable AV1 still-picture decoder: the seam issue #250 exposes so a user can hook in a
/// platform AV1 decoder (dav1d, VideoToolbox, VAAPI, MediaCodec, a hardware block …) and decode
/// compliant AVIF images end to end through this crate.
///
/// Implement [`decode_still`](Self::decode_still) once; the crate's pipeline
/// ([`AvifImage::decode_item_planar`] / [`AvifImage::decode_item_rgba8`]) then handles item
/// derivation, colour, alpha, and the transformative properties around it. The trait is
/// object-safe (`&mut dyn Av1StillDecoder`) and byte-slice-shaped so it is straightforward to
/// adapt across an FFI boundary — see the module documentation.
pub trait Av1StillDecoder {
    /// Decodes one AV1 still picture to a planar [`DecodedFrame`].
    ///
    /// `config` is the typed `av1C` record of the item (profile/level, bit depth, chroma, and any
    /// `configOBUs`); `payload` is the raw `av01` item payload — a low-overhead OBU stream. An
    /// implementation that needs one self-contained stream (a temporal delimiter, the
    /// `configOBUs`, then the payload, every OBU sized) builds it with
    /// [`Av1Config::full_stream`]; a decoder taking `av1C` + sample data separately (the shape
    /// hardware still-decode APIs use) passes the two through unchanged. The returned frame's
    /// `width`/`height` are the **post-superres** luma dimensions the decoder outputs.
    ///
    /// The pipeline guarantees `payload` has already passed
    /// [`Av1Config::validate_still_payload`] (exactly one sequence header, first frame a shown
    /// key frame, no tile list), so an implementation need not re-check the still-image
    /// constraints.
    ///
    /// # Errors
    ///
    /// Returns a [`gamut_core::Error`] if the codestream cannot be decoded — malformed bitstream,
    /// an unsupported AV1 tool or profile, and so on. The pipeline propagates it unchanged.
    fn decode_still(&mut self, config: &Av1Config, payload: &[u8]) -> Result<DecodedFrame>;
}

/// The owned planar output of an [`Av1StillDecoder`]: a YCbCr (or monochrome) frame at native
/// chroma and bit depth.
///
/// Samples are carried uniformly as `u16` whatever the `bit_depth` (an 8-bit frame simply uses
/// the low byte), so one plane layout serves 8..=16-bit content and maps cleanly onto a C buffer.
/// The planes are row-major: `y` is `width × height`; `cb`/`cr` are the chroma-plane dimensions
/// of the [`chroma`](Self::chroma) format (see [`ChromaFormat::chroma_dimensions`]) and are
/// **empty** for [`ChromaFormat::Monochrome`].
///
/// Construct with [`DecodedFrame::new`], which validates every plane length against
/// `width`/`height`/`chroma` (chroma dimensions use ceiling division, so an odd luma dimension is
/// handled) and the `bit_depth` range — an internally inconsistent frame is unrepresentable, so
/// the pipeline downstream can trust the plane sizes.
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
    /// `width`/`height` are the luma dimensions (post superres). The chroma planes `cb`/`cr` must
    /// match [`ChromaFormat::chroma_dimensions`] for `(width, height)` — which uses **ceiling**
    /// division on the subsampled axes, so a 5×3 luma frame in 4:2:0 has 3×2 chroma planes — and
    /// must both be **empty** for [`ChromaFormat::Monochrome`]. `bit_depth` must be in `8..=16`
    /// (AV1 itself codes 8/10/12; the wider bound keeps the frame type usable for intermediate
    /// results).
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
            .ok_or(Error::InvalidInput("AVIF: frame dimensions overflow usize"))?;
        if !(8..=16).contains(&bit_depth) {
            return Err(Error::InvalidInput(
                "AVIF: frame bit depth out of range (8..=16)",
            ));
        }
        if y.len() != luma {
            return Err(Error::InvalidInput(
                "AVIF: luma plane length does not match dimensions",
            ));
        }
        let (cw, ch) = chroma.chroma_dimensions(width, height);
        // Cannot overflow: each chroma dimension is at most its luma dimension, and `luma` fit
        // usize.
        let chroma_len = cw as usize * ch as usize;
        if chroma == ChromaFormat::Monochrome {
            if !cb.is_empty() || !cr.is_empty() {
                return Err(Error::InvalidInput(
                    "AVIF: monochrome frame must have empty chroma planes",
                ));
            }
        } else if cb.len() != chroma_len || cr.len() != chroma_len {
            return Err(Error::InvalidInput(
                "AVIF: chroma plane length does not match dimensions",
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

    /// The luma width in pixels (post superres).
    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }

    /// The luma height in pixels (post superres).
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

impl AvifImage {
    /// Decodes an item to a raw planar [`DecodedFrame`] — the surface that delivers everything,
    /// with no colour conversion or transformative properties applied (drive those from the
    /// item's [`AvifItem`](crate::AvifItem) accessors).
    ///
    /// The item is resolved through its derivation, recursively and depth-limited (a self- or
    /// mutually-referential `dimg` graph errors rather than recursing forever):
    ///
    /// - **Coded** (`av01`): the payload is validated against the AVIF still-image constraints
    ///   ([`Av1Config::validate_still_payload`]) and handed to `decoder`.
    /// - **`iden`**: the frame of its single `dimg` source.
    /// - **`grid`**: every tile is decoded (row-major `dimg` order) and assembled in the plane
    ///   domain onto a `columns·tile_w × rows·tile_h` canvas, then cropped to the grid's output
    ///   size (ISO/IEC 23008-12 §6.6.2.3.2). All tiles must share chroma format, bit depth, and
    ///   dimensions, else [`Error::Unsupported`]; the arithmetic is checked so a hostile grid
    ///   cannot amplify allocation.
    /// - **`iovl`**: [`Error::Unsupported`] — overlay compositing is defined over RGBA.
    ///
    /// Transformative properties (`clap`/`irot`/`imir`) are **not** applied here — read them via
    /// [`AvifItem::transformative_properties`](crate::AvifItem::transformative_properties).
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] for a missing item, a malformed `av1C`, a payload
    /// violating the still-image constraints, a derivation cycle, or a malformed derivation;
    /// [`Error::Unsupported`] for an `iovl`, a non-AV1 coded item, an item with an unrecognised
    /// essential property, a non-uniform grid, or a derivation nested past the internal depth
    /// limit; and propagates the `decoder`'s errors.
    pub fn decode_item_planar(
        &self,
        id: u32,
        decoder: &mut dyn Av1StillDecoder,
    ) -> Result<DecodedFrame> {
        let mut stack = Vec::new();
        self.decode_planar_inner(id, decoder, &mut stack)
    }

    // ---- planar pipeline ---------------------------------------------------------------------

    fn decode_planar_inner(
        &self,
        id: u32,
        decoder: &mut dyn Av1StillDecoder,
        stack: &mut Vec<u32>,
    ) -> Result<DecodedFrame> {
        let item = self
            .item(id)
            .ok_or(Error::InvalidInput("AVIF: item id names no item"))?;
        if item.has_unsupported_essential_property() {
            return Err(Error::Unsupported(
                "AVIF: item has an unrecognised essential property",
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
        item: AvifItem<'_>,
        decoder: &mut dyn Av1StillDecoder,
        stack: &mut Vec<u32>,
    ) -> Result<DecodedFrame> {
        match item.kind() {
            ItemKind::CodedImage { .. } if item.kind().is_av1() => {
                let config = item.av1_config().ok_or(Error::InvalidInput(
                    "AVIF: AV1 item has no av1C configuration",
                ))??;
                let payload = &item.as_isobmff_item().payload;
                // The pipeline enforces the still-image constraints before the codec hook sees
                // the payload (a non-conforming stream never reaches `decode_still`).
                config.validate_still_payload(payload)?;
                decoder.decode_still(&config, payload)
            }
            ItemKind::CodedImage { .. } => Err(Error::Unsupported(
                "AVIF: only AV1 coded items are decodable here",
            )),
            ItemKind::Identity => {
                let [source] = item.derivation_target_ids() else {
                    return Err(Error::InvalidInput(
                        "AVIF: iden item must reference exactly one source",
                    ));
                };
                self.decode_planar_inner(*source, decoder, stack)
            }
            ItemKind::Grid => self.decode_grid(id, item, decoder, stack),
            ItemKind::Overlay => Err(Error::Unsupported(
                "AVIF: overlay compositing is not available on the planar surface (use the RGBA path)",
            )),
            ItemKind::Exif | ItemKind::Mime | ItemKind::Unknown(_) => {
                Err(Error::InvalidInput("AVIF: item is not a decodable image"))
            }
        }
    }

    fn decode_grid(
        &self,
        id: u32,
        item: AvifItem<'_>,
        decoder: &mut dyn Av1StillDecoder,
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
                "AVIF: grid tiles are not uniform in dimensions, chroma, or bit depth",
            ));
        }

        // Canvas = columns·tile_w × rows·tile_h, checked; the output must fit within it.
        let canvas_w = (cols as u32)
            .checked_mul(tw)
            .ok_or(Error::InvalidInput("AVIF: grid canvas width overflow"))?;
        let canvas_h = (rows as u32)
            .checked_mul(th)
            .ok_or(Error::InvalidInput("AVIF: grid canvas height overflow"))?;
        let (ow, oh) = (grid.output_width, grid.output_height);
        if ow == 0 || oh == 0 || ow > canvas_w || oh > canvas_h {
            return Err(Error::InvalidInput(
                "AVIF: grid output dimensions exceed the tiled canvas",
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
}

// ---- derivation recursion guard --------------------------------------------------------------

/// Cycle- and depth-checks a derivation step, then records `id` on the recursion `stack`. The
/// caller pops `id` when the step completes.
fn enter_derivation(stack: &mut Vec<u32>, id: u32) -> Result<()> {
    if stack.contains(&id) {
        return Err(Error::InvalidInput("AVIF: derivation reference cycle"));
    }
    if stack.len() >= MAX_DERIVATION_DEPTH {
        return Err(Error::Unsupported("AVIF: derivation nested too deeply"));
    }
    stack.push(id);
    Ok(())
}

// ---- grid plane assembly ---------------------------------------------------------------------

/// Assembles one plane of a grid directly into its cropped output: for each output sample it
/// reads the covering tile's plane, so the tile canvas (`columns·tile_pw × rows·tile_ph`) is
/// sampled and cropped to `out_pw × out_ph` in a single pass. `out_pw <= columns·tile_pw` and
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
