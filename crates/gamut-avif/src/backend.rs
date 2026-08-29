//! The AV1 **still-encode** backend seam: the typed [`Av1StillEncoder`] trait a caller plugs an
//! alternate AV1 encoder into, the [`Av1EncodeRequest`] describing one encode job, and the
//! [`AbiAv1StillEncoder`] adapter that bridges a [`gamut_codec_abi::Encoder`] (and, through
//! [`gamut_codec_abi::bridge::ForeignEncoder`], a C/`-sys` backend) onto it.
//!
//! This is the write-direction mirror of the [`Av1StillDecoder`](crate::Av1StillDecoder) seam:
//! `gamut-avif` owns the container and everything around the coded picture, while the AV1
//! codestream itself may come from a platform or reference encoder (libaom, SVT-AV1, a hardware
//! encoder, …). The shape and the fallback contract are the workspace-wide ones defined by
//! [`gamut_codec_abi`] (issue #241/#272):
//!
//! - Backends are tried in **push order** ([`AvifEncoder::push_backend`](crate::AvifEncoder::push_backend)).
//! - [`Av1StillEncoder::supports`] returning `false` — or, across the C seam,
//!   [`Status::UNSUPPORTED`](gamut_codec_abi::Status::UNSUPPORTED) — is the **only** fall-through
//!   signal.
//! - `gamut-av1`'s [`encode_still_intra_with`](gamut_av1::encode_still_intra_with) is the
//!   **implicit tail**,
//!   used when every pushed backend declines. `gamut-av1` itself is unaware of this seam.
//! - A backend that *accepts* a job and then fails propagates its error; the tail is **not** retried,
//!   because silently substituting a different encoder would make the output non-deterministic.
//!
//! # The `av1C` record for a backend-supplied stream
//!
//! The container's `av1C`/`colr` boxes must mirror the sequence header the item payload actually
//! carries (AV1-ISOBMFF v1.3.0 §2.3.4). For the built-in tail those values come back from
//! `gamut-av1`; for a backend they are recovered from the returned OBUs themselves —
//! `seq_profile`, `seq_level_idx[0]`, the coded dimensions and `color_config()` are read from the
//! sequence header, the colour configuration is checked against the one the request asked for (so
//! the `colr` box can never disagree with the payload), and the stream is then checked against the
//! AVIF still-image item constraints
//! ([`Av1Config::validate_still_payload`](crate::Av1Config::validate_still_payload)). The rest of
//! the pixel parameters (8-bit, 4:4:4) are the v1 surface's fixed contract, stated on
//! [`Av1StillEncoder::encode_still`] and enforced by the `seq_profile` check.

use std::sync::{Arc, Mutex};

use gamut_av1::{Av1Colour, Av1StillConfig, EncodedStill};
use gamut_codec_abi::{EncodeConfig, Encoder, ImageDesc, Status};
use gamut_color::{BitDepth, ColorRange, Planar8, Planar16};
use gamut_core::{Dimensions, Error, PixelFormat, Result};

use crate::av1c::Av1Config;
use crate::obu::{ObuType, iter_obus};

/// The `codec_id` `gamut-avif` stamps into a [`EncodeConfig`] when it drives a
/// [`gamut_codec_abi`] backend: the big-endian `av01` FourCC — the same four bytes AV1-ISOBMFF
/// v1.3.0 §2.2 assigns the AV1 image item type — so a backend can dispatch on the codec without a
/// gamut-specific table.
pub const AV1_CODEC_ID: u32 = u32::from_be_bytes(*b"av01");

/// One AV1 still-encode job, as handed to an [`Av1StillEncoder`].
///
/// Constructed by `gamut-avif` (never by a caller) and read through its getters, so the crate can
/// grow the request without a breaking change. It carries the **already-derived**
/// [`base_q_idx`](Self::base_q_idx): the `quality → base_q_idx` mapping is part of the frozen v1
/// guarantee (see `STATUS.md`) and lives inside the encoder, so a backend never sees — and can
/// never reinterpret — the `0..=100` quality scale.
///
/// `#[non_exhaustive]`, so the deferred pixel-format work (M2: 10/12-bit, 4:2:0/4:2:2, limited
/// range) extends it additively.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct Av1EncodeRequest {
    /// Display dimensions of the image to encode.
    dimensions: Dimensions,
    /// The AV1 `base_q_idx` (AV1 §5.9.12), `0..=255`; `0` selects the lossless path.
    base_q_idx: u8,
    /// The colour signalling the returned stream must carry — and the layout `planes` is in.
    colour: Av1Colour,
    /// The depth the samples are coded at: 8, 10 or 12 bits.
    bit_depth: BitDepth,
}

impl Av1EncodeRequest {
    /// Builds a request. Crate-internal: the `base_q_idx` must already have been derived through
    /// the encoder's frozen quality mapping.
    pub(crate) fn new(
        dimensions: Dimensions,
        base_q_idx: u8,
        colour: Av1Colour,
        bit_depth: BitDepth,
    ) -> Self {
        Self {
            dimensions,
            base_q_idx,
            colour,
            bit_depth,
        }
    }

    /// The display dimensions of the image to encode.
    #[must_use]
    pub fn dimensions(&self) -> Dimensions {
        self.dimensions
    }

    /// The display width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        self.dimensions.width
    }

    /// The display height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        self.dimensions.height
    }

    /// The AV1 `base_q_idx` (AV1 §5.9.12) to encode at, `0..=255` — lower is finer. This is the
    /// authoritative quantizer for the job; it is already derived from the encoder's quality
    /// setting, which backends do not see.
    #[must_use]
    pub fn base_q_idx(&self) -> u8 {
        self.base_q_idx
    }

    /// Whether this is a **lossless** encode — i.e. [`base_q_idx`](Self::base_q_idx) is `0`, the
    /// AV1 lossless path, for which the decoded output must be bit-exact to the input.
    #[must_use]
    pub fn is_lossless(&self) -> bool {
        self.base_q_idx == 0
    }

    /// The colour encoding the job is in: the CICP triple plus the signal range.
    ///
    /// This is **both** an input and an obligation. It says what the `planes` handed to
    /// [`Av1StillEncoder::encode_still`] contain — GBR for
    /// [`MatrixCoefficients::Identity`](gamut_color::MatrixCoefficients::Identity), `Y/Cb/Cr`
    /// otherwise — and the returned stream's `color_config()` must declare exactly it, because the
    /// container mirrors these values into `colr` and they must agree with the payload
    /// (AV1-ISOBMFF v1.3.0 §2.3.4). A mismatch is rejected.
    #[must_use]
    pub fn colour(&self) -> Av1Colour {
        self.colour
    }

    /// The depth the samples are coded at.
    ///
    /// [`BitDepth::Eight`] is the only depth [`Av1StillEncoder::encode_still`] is ever handed;
    /// [`BitDepth::Ten`] and [`BitDepth::Twelve`] arrive through
    /// [`encode_still16`](Av1StillEncoder::encode_still16), whose default implementation declines.
    #[must_use]
    pub fn bit_depth(&self) -> BitDepth {
        self.bit_depth
    }
}

/// A pluggable AV1 **still-image encoder** backend.
///
/// Implement this to route [`AvifEncoder`](crate::AvifEncoder)'s codestream through an alternate
/// AV1 encoder (a platform/hardware encoder, libaom, SVT-AV1, …) and register it with
/// [`AvifEncoder::push_backend`](crate::AvifEncoder::push_backend). A C or `-sys` backend reaches
/// this trait through [`AbiAv1StillEncoder`] rather than implementing it directly.
///
/// `Send` is a supertrait because the registry stores backends behind an
/// [`Arc`]`<`[`Mutex`]`<…>>` so [`AvifEncoder`](crate::AvifEncoder) stays [`Clone`] and encodes
/// through `&self`.
pub trait Av1StillEncoder: Send {
    /// Reports whether this backend can satisfy `req`. Returning `false` is the **only** signal
    /// that lets the encoder fall through to the next backend (and finally to the built-in
    /// `gamut-av1` tail); a backend that returns `true` owns the job outright.
    fn supports(&mut self, req: &Av1EncodeRequest) -> bool;

    /// Encodes `planes` and returns the **AV1 OBU byte stream** for the `av01` item payload — the
    /// low-overhead OBU syntax of AV1 §5.3, with no temporal delimiter, exactly as
    /// [`gamut_av1::EncodedStill::obus`] carries it.
    ///
    /// `planes` are `gamut-color` planes, each `width * height` 8-bit samples, in the layout
    /// [`Av1EncodeRequest::colour`] describes: identity GBR (`Y = G`, `U = B`, `V = R`) or
    /// `Y/Cb/Cr` through that matrix. The v1 surface fixes the rest of the coding parameters: the
    /// returned stream must be a **still picture** with `seq_profile = 1` (High), 8-bit, 4:4:4,
    /// whose sequence header carries `reduced_still_picture_header = 1`, the request's dimensions,
    /// and a `color_config()` matching the request's colour. The crate re-derives the
    /// `av1C`/`colr` boxes from that sequence header and rejects a stream that does not meet the
    /// contract.
    ///
    /// # Errors
    ///
    /// Any error a backend returns after accepting the job **propagates** to the caller of
    /// [`encode_image`](gamut_core::EncodeImage::encode_image); the built-in encoder is not used
    /// as a silent fallback. Decline the job from [`supports`](Self::supports) instead.
    fn encode_still(&mut self, req: &Av1EncodeRequest, planes: &Planar8) -> Result<Vec<u8>>;

    /// Encodes **10- or 12-bit** planes, returning the AV1 OBU byte stream exactly as
    /// [`encode_still`](Self::encode_still) does for 8-bit.
    ///
    /// The default implementation **declines**, and the host falls through to the next backend and
    /// finally to the built-in `gamut-av1` tail — so a backend written against the 8-bit v1
    /// contract keeps compiling *and* keeps its meaning: it is never handed samples it did not
    /// agree to encode. Override it to opt into high bit depth.
    ///
    /// Declining here is the one late fall-through the seam allows, and it exists because
    /// [`supports`](Self::supports) is a single answer for a job whose depth a pre-existing
    /// implementation does not inspect. Every *other* failure after accepting a job still
    /// propagates.
    ///
    /// The returned stream's `seq_profile` must match the depth §6.4.1 requires — 1 (or 0
    /// monochrome) at 10 bits, 2 at 12 — and its `color_config()` must declare
    /// [`Av1EncodeRequest::bit_depth`] and [`Av1EncodeRequest::colour`]; the crate re-derives the
    /// `av1C`/`colr` boxes from it and rejects a disagreement.
    ///
    /// # Errors
    ///
    /// As [`encode_still`](Self::encode_still).
    fn encode_still16(&mut self, req: &Av1EncodeRequest, planes: &Planar16) -> Result<Vec<u8>> {
        let _ = (req, planes);
        Err(LateDecline::error())
    }
}

/// The registry entry type: a shared, interior-mutable backend.
///
/// [`Arc`] so cloning an [`AvifEncoder`](crate::AvifEncoder) **shares** backends rather than
/// duplicating (a backend is typically a stateful, non-`Clone` encoder handle), and [`Mutex`] so
/// `&self` encoding can call the `&mut self` trait methods.
pub(crate) type BackendSlot = Arc<Mutex<dyn Av1StillEncoder + Send>>;

/// The planes one registry run offers a backend, at whichever width the job actually has.
pub(crate) enum BackendPlanes<'a> {
    /// 8-bit planes, offered through [`Av1StillEncoder::encode_still`].
    Eight(&'a Planar8),
    /// 10/12-bit planes, offered through [`Av1StillEncoder::encode_still16`].
    High(&'a Planar16),
}

/// Runs the registry for one request: tries `backends` in push order, returning the first
/// acceptance's OBUs, or `None` when every backend declines (the caller then uses the built-in
/// `gamut-av1` tail).
pub(crate) fn run_backends(
    backends: &[BackendSlot],
    req: &Av1EncodeRequest,
    planes: BackendPlanes<'_>,
) -> Result<Option<Vec<u8>>> {
    for slot in backends {
        let mut backend = slot.lock().map_err(|_| {
            Error::invalid_input(
                env!("CARGO_PKG_NAME"),
                "AVIF: AV1 encode backend is poisoned",
            )
        })?;
        if backend.supports(req) {
            // Accepted: this backend owns the job, so its error propagates — falling back to a
            // different encoder here would silently change the output bytes. The sole exception is
            // the ABI adapter's late-`UNSUPPORTED` sentinel, which *is* a decline.
            let encoded = match planes {
                BackendPlanes::Eight(p) => backend.encode_still(req, p),
                BackendPlanes::High(p) => backend.encode_still16(req, p),
            };
            return match encoded {
                Err(e) if LateDecline::is(&e) => continue,
                other => other.map(Some),
            };
        }
    }
    Ok(None)
}

/// Rebuilds the [`EncodedStill`] — OBUs plus the sequence-header values the container stamps into
/// `av1C`/`colr` — for a stream a backend produced, validating it against the v1 contract and the
/// AVIF still-image item constraints.
///
/// # Errors
///
/// - [`Error::InvalidInput`] if the stream has no parsable reduced-still-picture sequence header,
///   or if its coded dimensions disagree with `dims` (which the container's `ispe` states).
/// - [`Error::Unsupported`] if the sequence header is not `reduced_still_picture_header = 1`, or
///   its `seq_profile`/depth pair is not one this surface can describe.
/// - [`Error::InvalidInput`] if the stream's coded depth is not the one the request asked for —
///   the container's `av1C` and `pixi` would otherwise lie about the payload.
/// - Whatever [`Av1Config::validate_still_payload`] reports for a non-conformant item payload.
pub(crate) fn still_from_backend_obus(
    obus: Vec<u8>,
    dims: Dimensions,
    colour: Av1Colour,
    bit_depth: BitDepth,
) -> Result<EncodedStill> {
    let header = SeqHeaderParams::parse(&obus)?;
    if header.bit_depth != bit_depth {
        return Err(Error::invalid_input(
            env!("CARGO_PKG_NAME"),
            "AVIF: AV1 backend stream is coded at a different bit depth than requested",
        ));
    }
    if (header.width, header.height) != (dims.width, dims.height) {
        return Err(Error::invalid_input(
            env!("CARGO_PKG_NAME"),
            "AVIF: AV1 backend stream dimensions differ from the image",
        ));
    }
    // The container mirrors these into `colr`, so they must be what the payload actually declares
    // — not what the request asked for. Read them back and reject a disagreement rather than stamp
    // a `colr` box that lies about the samples (AV1-ISOBMFF v1.3.0 §2.3.4).
    let requested = (
        colour.primaries.code_point(),
        colour.transfer.code_point(),
        colour.matrix.code_point(),
        matches!(colour.range, ColorRange::Full),
    );
    if header.colour != requested {
        return Err(Error::invalid_input(
            env!("CARGO_PKG_NAME"),
            "AVIF: AV1 backend stream signals a different colour configuration than requested",
        ));
    }
    let (color_primaries, transfer_characteristics, matrix_coefficients, full_range) =
        header.colour;
    let config = Av1StillConfig {
        seq_profile: header.seq_profile,
        seq_level_idx_0: header.seq_level_idx_0,
        seq_tier_0: 0,
        high_bitdepth: bit_depth != BitDepth::Eight,
        twelve_bit: bit_depth == BitDepth::Twelve,
        monochrome: false,
        chroma_subsampling_x: 0,
        chroma_subsampling_y: 0,
        chroma_sample_position: 0,
        color_primaries,
        transfer_characteristics,
        matrix_coefficients,
        full_range,
    };
    // Re-read the record we are about to stamp and validate the payload through the crate's own
    // reader, so a backend stream faces exactly the checks a third-party AVIF file would.
    Av1Config::parse(&crate::encoder::av1c_record(&config))?.validate_still_payload(&obus)?;
    Ok(EncodedStill { obus, config })
}

/// The sequence-header fields the container needs from a backend-supplied stream.
struct SeqHeaderParams {
    /// `seq_profile` (3 bits).
    seq_profile: u8,
    /// `seq_level_idx[0]` (5 bits).
    seq_level_idx_0: u8,
    /// `max_frame_width_minus_1 + 1`.
    width: u32,
    /// `max_frame_height_minus_1 + 1`.
    height: u32,
    /// `color_config()`: `(color_primaries, transfer_characteristics, matrix_coefficients,
    /// color_range == full)`.
    colour: (u16, u16, u16, bool),
    /// `BitDepth`, as §5.5.2 derives it from `high_bitdepth`, `twelve_bit` and `seq_profile`.
    bit_depth: BitDepth,
}

impl SeqHeaderParams {
    /// Reads the (single) sequence header OBU in `payload`, through `color_config()`.
    ///
    /// Only the `reduced_still_picture_header = 1` layout is read — the shape AVIF still images
    /// use (AV1 §5.5.1: with the reduced header the syntax runs `seq_profile(3)`,
    /// `still_picture(1)`, `reduced_still_picture_header(1)`, `seq_level_idx[0](5)`,
    /// `frame_width_bits_minus_1(4)`, `frame_height_bits_minus_1(4)`,
    /// `max_frame_width_minus_1(n)`, `max_frame_height_minus_1(m)`, then `use_128x128_superblock`,
    /// `enable_filter_intra`, `enable_intra_edge_filter`, `enable_superres`, `enable_cdef`,
    /// `enable_restoration`, and `color_config()`).
    fn parse(payload: &[u8]) -> Result<Self> {
        let seq = iter_obus(payload)
            .find_map(|obu| match obu {
                Ok(obu) if obu.header.obu_type == ObuType::SequenceHeader => Some(Ok(obu.payload)),
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            })
            .ok_or_else(|| {
                Error::invalid_input(
                    env!("CARGO_PKG_NAME"),
                    "AVIF: AV1 backend stream has no sequence header OBU",
                )
            })??;
        let mut r = BitReader::new(seq);
        let seq_profile = r.bits(3)? as u8;
        // Checked here, not by the caller, because `color_config()`'s *layout* depends on it: only
        // `seq_profile == 2` codes `twelve_bit`, and only `seq_profile != 1` codes `mono_chrome`.
        // Reading the colour fields off a profile this parser cannot describe would misparse them
        // before anyone had a chance to reject the stream.
        //
        // Profile 0 is admitted so far as the *syntax* goes — it is what a monochrome stream uses —
        // but the `mono_chrome` bit below is then rejected, because the container surface this
        // rebuilds an `av1C` for describes a three-plane colour item.
        if seq_profile > 2 {
            return Err(Error::unsupported(
                env!("CARGO_PKG_NAME"),
                "AVIF: AV1 backend stream must use seq_profile 0, 1 or 2",
            ));
        }
        let _still_picture = r.bits(1)?;
        if r.bits(1)? != 1 {
            return Err(Error::unsupported(
                env!("CARGO_PKG_NAME"),
                "AVIF: AV1 backend stream must set reduced_still_picture_header",
            ));
        }
        let seq_level_idx_0 = r.bits(5)? as u8;
        let width_bits = r.bits(4)? + 1;
        let height_bits = r.bits(4)? + 1;
        let width = r.bits(width_bits)? + 1;
        let height = r.bits(height_bits)? + 1;
        // `frame_id_numbers_present_flag` is inferred 0 under the reduced header; the six enable
        // flags below carry no information the container needs, but they must be stepped over to
        // reach `color_config()`.
        r.bits(6)?; // use_128x128_superblock, filter_intra, intra_edge_filter, superres, cdef, restoration

        // color_config() (§5.5.2): `high_bitdepth`, then `twelve_bit` only under profile 2, then
        // `mono_chrome` for every profile but 1.
        let high_bitdepth = r.bits(1)? != 0;
        let twelve_bit = seq_profile == 2 && high_bitdepth && r.bits(1)? != 0;
        let bit_depth = match (high_bitdepth, twelve_bit) {
            (false, _) => BitDepth::Eight,
            (true, false) => BitDepth::Ten,
            (true, true) => BitDepth::Twelve,
        };
        if seq_profile != 1 && r.bits(1)? != 0 {
            return Err(Error::unsupported(
                env!("CARGO_PKG_NAME"),
                "AVIF: AV1 backend stream must be three-plane (mono_chrome = 0)",
            ));
        }
        // `color_description_present_flag = 0` leaves all three code points UNSPECIFIED (2), which
        // the container would have to stamp verbatim into `colr` — and which can never match the
        // concrete triple the request carries. Reject it here, with a message that says why,
        // rather than let it fall out of the colour comparison as a confusing mismatch.
        if r.bits(1)? != 1 {
            return Err(Error::unsupported(
                env!("CARGO_PKG_NAME"),
                "AVIF: AV1 backend stream must set color_description_present_flag",
            ));
        }
        let cp = r.bits(8)? as u16;
        let tc = r.bits(8)? as u16;
        let mc = r.bits(8)? as u16;
        // The §5.5.2 shortcut: BT.709 primaries + sRGB transfer + identity matrix infer full range
        // (and 4:4:4) with no coded bit; every other triple codes `color_range`.
        let shortcut = cp == 1 && tc == 13 && mc == 0;
        let full_range = if shortcut { true } else { r.bits(1)? == 1 };
        // §5.5.2 *derives* the subsampling from `seq_profile` rather than always coding it, so a
        // stream can be subsampled without a bit here saying so: profile 0 is 4:2:0, profile 2 is
        // 4:2:2 below 12-bit, and only profile 2 at 12-bit codes the pair. `still_from_backend_obus`
        // rebuilds an `av1C` that declares 4:4:4 unconditionally, so anything else has to be
        // refused *here* — `validate_still_payload` checks the record against the OBU structure,
        // not against the sequence header's colour fields, and would not catch the disagreement.
        let (subsampling_x, subsampling_y) = if shortcut {
            // The shortcut infers 4:4:4 whatever the profile says.
            (0, 0)
        } else {
            match seq_profile {
                0 => (1, 1),
                1 => (0, 0),
                // Profile 2: 4:2:2 at 8/10-bit; at 12-bit the pair is coded, and 4:4:4 is the one
                // combination this container can describe.
                _ if bit_depth == BitDepth::Twelve => {
                    let sx = r.bits(1)?;
                    let sy = if sx == 1 { r.bits(1)? } else { 0 };
                    (sx as u8, sy as u8)
                }
                _ => (1, 0),
            }
        };
        if (subsampling_x, subsampling_y) != (0, 0) {
            // Name the layout the header implies, not just the one required: the caller has to fix
            // their encoder's configuration, and `subsampling_y` is what separates the two cases.
            return Err(Error::unsupported(
                env!("CARGO_PKG_NAME"),
                if subsampling_y == 1 {
                    "AVIF: AV1 backend stream must be 4:4:4; its sequence header implies 4:2:0"
                } else {
                    "AVIF: AV1 backend stream must be 4:4:4; its sequence header implies 4:2:2"
                },
            ));
        }
        let colour = (cp, tc, mc, full_range);
        Ok(Self {
            bit_depth,
            seq_profile,
            seq_level_idx_0,
            width,
            height,
            colour,
        })
    }
}

/// A minimal MSB-first bit reader over an OBU payload (AV1 §4.10.2 `f(n)`).
struct BitReader<'a> {
    /// The bytes being read.
    data: &'a [u8],
    /// Bit cursor, counted from the MSB of `data[0]`.
    pos: u32,
}

impl<'a> BitReader<'a> {
    /// Starts a reader at bit 0 of `data`.
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Reads the next `n` bits (`n <= 32`) MSB-first.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if fewer than `n` bits remain.
    fn bits(&mut self, n: u32) -> Result<u32> {
        let mut value = 0u32;
        for _ in 0..n {
            let byte = self.data.get((self.pos / 8) as usize).ok_or_else(|| {
                Error::invalid_input(
                    env!("CARGO_PKG_NAME"),
                    "AVIF: AV1 backend sequence header truncated",
                )
            })?;
            let bit = (byte >> (7 - self.pos % 8)) & 1;
            value = (value << 1) | u32::from(bit);
            self.pos += 1;
        }
        Ok(value)
    }
}

/// Bridges a [`gamut_codec_abi::Encoder`] — the workspace-wide codestream seam, and hence any C /
/// `-sys` backend reached through [`gamut_codec_abi::bridge::ForeignEncoder`] — onto
/// [`Av1StillEncoder`].
///
/// The adapter lowers an [`Av1EncodeRequest`] into an [`EncodeConfig`] tagged with
/// [`AV1_CODEC_ID`], presents the [`Planar8`] input as a three-plane [`ImageDesc`], and collects
/// the encoder's sink chunks into the returned OBU buffer.
///
/// # The quantizer, not the quality
///
/// [`EncodeConfig::quality`] is a `0..=100` scale, but `gamut-avif`'s `quality → base_q_idx`
/// mapping is frozen and one-way: synthesizing a `0..=100` value back out of the derived
/// `base_q_idx` would invent a second, contradictory mapping. The adapter therefore leaves
/// `quality` at `0` and passes the authoritative quantizer out-of-band, as the codec-specific
/// [`EncodeConfig::extra`] blob: **one byte**, the AV1 `base_q_idx`. A backend registered for
/// [`AV1_CODEC_ID`] must read `extra`, and must not read `quality`.
///
/// # Bit depth: 8-bit only
///
/// The adapter implements [`Av1StillEncoder::encode_still`] and **not**
/// [`encode_still16`](Av1StillEncoder::encode_still16), so a 10- or 12-bit job never reaches the
/// wrapped ABI encoder: the trait default declines it and the registry falls through to the next
/// backend, ultimately the built-in software tail. A caller who wraps a hardware or `-sys` encoder
/// and then encodes an `Rgb16`/`Rgba16` source therefore gets a correct file produced by
/// `gamut-av1`, not by the backend they registered.
///
/// This is a limitation of the adapter, not of the seam: [`ImageDesc`] carries a `bit_depth`, so
/// lowering `Planar16` across the ABI is expressible and is deferred additive work. Implement
/// [`Av1StillEncoder`] directly to take high-bit-depth jobs today.
///
/// # Status handling
///
/// [`Status::UNSUPPORTED`] is the fall-through code in both places it can appear: from `supports`
/// (as `false`) and — for a backend that changes its mind — from `encode`, where it is treated as
/// a late decline and the encoder moves on to the next backend. Any other non-OK status is a
/// terminal failure and propagates as [`Error::InvalidInput`].
pub struct AbiAv1StillEncoder<E: Encoder + Send> {
    /// The wrapped ABI encoder.
    inner: E,
}

impl<E: Encoder + Send> AbiAv1StillEncoder<E> {
    /// Wraps a [`gamut_codec_abi::Encoder`] as an [`Av1StillEncoder`].
    #[must_use]
    pub fn new(inner: E) -> Self {
        Self { inner }
    }

    /// Consumes the adapter, returning the wrapped encoder.
    #[must_use]
    pub fn into_inner(self) -> E {
        self.inner
    }

    /// The [`EncodeConfig`] for a job: [`AV1_CODEC_ID`], `quality = 0`, and `extra` pointing at
    /// `q_idx` (the request's `base_q_idx`, borrowed for the duration of the call).
    fn config(q_idx: &u8) -> EncodeConfig {
        let mut cfg = EncodeConfig::new(AV1_CODEC_ID, 0);
        cfg.extra = std::ptr::from_ref(q_idx).cast();
        cfg.extra_len = 1;
        cfg
    }
}

impl<E: Encoder + Send> std::fmt::Debug for AbiAv1StillEncoder<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AbiAv1StillEncoder").finish_non_exhaustive()
    }
}

impl<E: Encoder + Send> Av1StillEncoder for AbiAv1StillEncoder<E> {
    fn supports(&mut self, req: &Av1EncodeRequest) -> bool {
        let q_idx = req.base_q_idx();
        self.inner.supports(&Self::config(&q_idx))
    }

    fn encode_still(&mut self, req: &Av1EncodeRequest, planes: &Planar8) -> Result<Vec<u8>> {
        let q_idx = req.base_q_idx();
        let cfg = Self::config(&q_idx);
        // Per plane, not one luma stride for all three: `Planar8` carries its chroma geometry, and
        // `ImageDesc` has a stride slot per plane precisely so a subsampled buffer is expressible.
        // At 4:4:4 — everything this crate encodes today — all three are the luma width.
        let stride = |i: usize| planes.plane_dimensions(i).0 as usize;
        // Encode inputs are read-only per `ImageDesc`'s contract; the `*mut` is the ABI's single
        // descriptor shape shared with the decode (write) direction.
        let plane_ptr = |i: usize| planes.plane(i).as_ptr().cast_mut();
        let image = ImageDesc::new(
            // Three 8-bit planes in gamut's identity order (Y = G, U = B, V = R) — the planar
            // form of `PixelFormat::Rgb8`, which is what the AVIF v1 surface encodes.
            PixelFormat::Rgb8 as u32,
            planes.width(),
            planes.height(),
            8,
            3,
            [
                plane_ptr(0),
                plane_ptr(1),
                plane_ptr(2),
                std::ptr::null_mut(),
            ],
            [stride(0), stride(1), stride(2), 0],
        );
        let mut obus = Vec::new();
        let status = self.inner.encode(&cfg, &image, &mut |chunk: &[u8]| {
            obus.extend_from_slice(chunk);
            Status::OK
        });
        if status.is_ok() {
            Ok(obus)
        } else if status.is_unsupported() {
            Err(LateDecline::error().with_detail(format!("codec-abi status {}", status.0)))
        } else {
            Err(
                Error::invalid_input(env!("CARGO_PKG_NAME"), "AVIF: AV1 encode backend failed")
                    .with_detail(format!("codec-abi status {}", status.0)),
            )
        }
    }
}

/// The sentinel for a **late** decline — a backend that answered `supports` affirmatively and then
/// declined at `encode` time.
///
/// The typed trait has no third outcome between "handled" and "declined", so the late decline is
/// this exact [`Error::Unsupported`] payload, which [`run_backends`] maps back to a fall-through.
/// Two things raise it: [`AbiAv1StillEncoder`] translating a late [`Status::UNSUPPORTED`] from a C
/// backend, and the default [`Av1StillEncoder::encode_still16`], which is how a backend written
/// against the 8-bit contract declines a high-bit-depth job it never agreed to. A hand-written
/// backend otherwise signals a decline from [`supports`](Av1StillEncoder::supports).
pub(crate) struct LateDecline;

impl LateDecline {
    /// The sentinel error value.
    pub(crate) const MESSAGE: &'static str = "AVIF: AV1 encode backend declined late (UNSUPPORTED)";
    /// Builds the sentinel error.
    pub(crate) fn error() -> Error {
        Error::unsupported(env!("CARGO_PKG_NAME"), Self::MESSAGE)
    }

    /// Whether `err` is the late-decline sentinel.
    pub(crate) fn is(err: &Error) -> bool {
        err.kind() == gamut_core::ErrorKind::Unsupported
            && err.static_message() == Some(Self::MESSAGE)
    }
}
