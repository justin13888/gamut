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
//! - `gamut-av1`'s [`encode_still_intra`](gamut_av1::encode_still_intra) is the **implicit tail**,
//!   used when every pushed backend declines. `gamut-av1` itself is unaware of this seam.
//! - A backend that *accepts* a job and then fails propagates its error; the tail is **not** retried,
//!   because silently substituting a different encoder would make the output non-deterministic.
//!
//! # The `av1C` record for a backend-supplied stream
//!
//! The container's `av1C`/`colr` boxes must mirror the sequence header the item payload actually
//! carries (AV1-ISOBMFF v1.3.0 §2.3.4). For the built-in tail those values come back from
//! `gamut-av1`; for a backend they are recovered from the returned OBUs themselves —
//! `seq_profile`, `seq_level_idx[0]` and the coded dimensions are read from the sequence header,
//! and the stream is then checked against the AVIF still-image item constraints
//! ([`Av1Config::validate_still_payload`](crate::Av1Config::validate_still_payload)). The pixel
//! parameters (8-bit, identity 4:4:4, full range) are the v1 surface's fixed contract, stated on
//! [`Av1StillEncoder::encode_still`] and enforced by the `seq_profile` check.

use std::sync::{Arc, Mutex};

use gamut_av1::{Av1StillConfig, EncodedStill};
use gamut_codec_abi::{EncodeConfig, Encoder, ImageDesc, Status};
use gamut_color::{ColourPrimaries, MatrixCoefficients, Planar8, TransferCharacteristics};
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
}

impl Av1EncodeRequest {
    /// Builds a request. Crate-internal: the `base_q_idx` must already have been derived through
    /// the encoder's frozen quality mapping.
    pub(crate) fn new(dimensions: Dimensions, base_q_idx: u8) -> Self {
        Self {
            dimensions,
            base_q_idx,
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
    /// `planes` are `gamut-color` identity planes (`Y = G`, `U = B`, `V = R`), each
    /// `width * height` 8-bit samples. The v1 surface fixes the coding parameters: the returned
    /// stream must be a **still picture** with `seq_profile = 1` (High), 8-bit, 4:4:4, full range,
    /// identity matrix, whose sequence header carries `reduced_still_picture_header = 1` and the
    /// request's dimensions. The crate re-derives the `av1C`/`colr` boxes from that sequence
    /// header and rejects a stream that does not meet the contract.
    ///
    /// # Errors
    ///
    /// Any error a backend returns after accepting the job **propagates** to the caller of
    /// [`encode_image`](gamut_core::EncodeImage::encode_image); the built-in encoder is not used
    /// as a silent fallback. Decline the job from [`supports`](Self::supports) instead.
    fn encode_still(&mut self, req: &Av1EncodeRequest, planes: &Planar8) -> Result<Vec<u8>>;
}

/// The registry entry type: a shared, interior-mutable backend.
///
/// [`Arc`] so cloning an [`AvifEncoder`](crate::AvifEncoder) **shares** backends rather than
/// duplicating (a backend is typically a stateful, non-`Clone` encoder handle), and [`Mutex`] so
/// `&self` encoding can call the `&mut self` trait methods.
pub(crate) type BackendSlot = Arc<Mutex<dyn Av1StillEncoder + Send>>;

/// Runs the registry for one request: tries `backends` in push order, returning the first
/// acceptance's OBUs, or `None` when every backend declines (the caller then uses the built-in
/// `gamut-av1` tail).
pub(crate) fn run_backends(
    backends: &[BackendSlot],
    req: &Av1EncodeRequest,
    planes: &Planar8,
) -> Result<Option<Vec<u8>>> {
    for slot in backends {
        let mut backend = slot
            .lock()
            .map_err(|_| Error::InvalidInput("AVIF: AV1 encode backend is poisoned"))?;
        if backend.supports(req) {
            // Accepted: this backend owns the job, so its error propagates — falling back to a
            // different encoder here would silently change the output bytes. The sole exception is
            // the ABI adapter's late-`UNSUPPORTED` sentinel, which *is* a decline.
            return match backend.encode_still(req, planes) {
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
/// - [`Error::Unsupported`] if the sequence header is not `seq_profile = 1` /
///   `reduced_still_picture_header = 1` (the only shape the v1 surface can describe).
/// - Whatever [`Av1Config::validate_still_payload`] reports for a non-conformant item payload.
pub(crate) fn still_from_backend_obus(obus: Vec<u8>, dims: Dimensions) -> Result<EncodedStill> {
    let header = SeqHeaderParams::parse(&obus)?;
    if header.seq_profile != 1 {
        return Err(Error::Unsupported(
            "AVIF: AV1 backend stream must use seq_profile 1 (8-bit 4:4:4)",
        ));
    }
    if (header.width, header.height) != (dims.width, dims.height) {
        return Err(Error::InvalidInput(
            "AVIF: AV1 backend stream dimensions differ from the image",
        ));
    }
    let config = Av1StillConfig {
        seq_profile: header.seq_profile,
        seq_level_idx_0: header.seq_level_idx_0,
        seq_tier_0: 0,
        high_bitdepth: false,
        twelve_bit: false,
        monochrome: false,
        chroma_subsampling_x: 0,
        chroma_subsampling_y: 0,
        chroma_sample_position: 0,
        color_primaries: ColourPrimaries::Bt709.code_point(),
        transfer_characteristics: TransferCharacteristics::Srgb.code_point(),
        matrix_coefficients: MatrixCoefficients::Identity.code_point(),
        full_range: true,
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
}

impl SeqHeaderParams {
    /// Reads the leading fields of the (single) sequence header OBU in `payload`.
    ///
    /// Only the `reduced_still_picture_header = 1` layout is read — the shape AVIF still images
    /// use (AV1 §5.5.1: with the reduced header the syntax runs `seq_profile(3)`,
    /// `still_picture(1)`, `reduced_still_picture_header(1)`, `seq_level_idx[0](5)`,
    /// `frame_width_bits_minus_1(4)`, `frame_height_bits_minus_1(4)`,
    /// `max_frame_width_minus_1(n)`, `max_frame_height_minus_1(m)`).
    fn parse(payload: &[u8]) -> Result<Self> {
        let seq = iter_obus(payload)
            .find_map(|obu| match obu {
                Ok(obu) if obu.header.obu_type == ObuType::SequenceHeader => Some(Ok(obu.payload)),
                Ok(_) => None,
                Err(e) => Some(Err(e)),
            })
            .ok_or(Error::InvalidInput(
                "AVIF: AV1 backend stream has no sequence header OBU",
            ))??;
        let mut r = BitReader::new(seq);
        let seq_profile = r.bits(3)? as u8;
        let _still_picture = r.bits(1)?;
        if r.bits(1)? != 1 {
            return Err(Error::Unsupported(
                "AVIF: AV1 backend stream must set reduced_still_picture_header",
            ));
        }
        let seq_level_idx_0 = r.bits(5)? as u8;
        let width_bits = r.bits(4)? + 1;
        let height_bits = r.bits(4)? + 1;
        let width = r.bits(width_bits)? + 1;
        let height = r.bits(height_bits)? + 1;
        Ok(Self {
            seq_profile,
            seq_level_idx_0,
            width,
            height,
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
            let byte = self
                .data
                .get((self.pos / 8) as usize)
                .ok_or(Error::InvalidInput(
                    "AVIF: AV1 backend sequence header truncated",
                ))?;
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
        let stride = planes.width() as usize;
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
            [stride, stride, stride, 0],
        );
        let mut obus = Vec::new();
        let status = self.inner.encode(&cfg, &image, &mut |chunk: &[u8]| {
            obus.extend_from_slice(chunk);
            Status::OK
        });
        if status.is_ok() {
            Ok(obus)
        } else if status.is_unsupported() {
            Err(LateDecline::ERROR)
        } else {
            Err(Error::InvalidInput("AVIF: AV1 encode backend failed"))
        }
    }
}

/// The sentinel by which [`AbiAv1StillEncoder`] reports a **late** [`Status::UNSUPPORTED`] — a
/// backend that answered `supports` affirmatively and then declined at `encode` time.
///
/// The typed trait has no third outcome between "handled" and "declined", so the adapter encodes
/// the late decline as this exact [`Error::Unsupported`] payload, which [`run_backends`] maps back
/// to a fall-through. It is an internal protocol between the two: a hand-written
/// [`Av1StillEncoder`] signals a decline from [`supports`](Av1StillEncoder::supports).
pub(crate) struct LateDecline;

impl LateDecline {
    /// The sentinel error value.
    pub(crate) const MESSAGE: &'static str = "AVIF: AV1 encode backend declined late (UNSUPPORTED)";
    /// The sentinel error.
    pub(crate) const ERROR: Error = Error::Unsupported(Self::MESSAGE);

    /// Whether `err` is the late-decline sentinel.
    pub(crate) fn is(err: &Error) -> bool {
        matches!(err, Error::Unsupported(m) if *m == Self::MESSAGE)
    }
}
