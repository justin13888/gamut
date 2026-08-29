//! The AV1 still-encode backend seam (issue #274): the 1.0 additivity guarantee (an encoder with
//! no backend is byte-identical to the pre-backend encoder), the push-order fallback contract, and
//! the `gamut-codec-abi` adapter.

use std::sync::{Arc, Mutex};

use gamut_av1::Av1Colour;
use gamut_avif::{
    AbiAv1StillEncoder, Av1EncodeRequest, Av1StillEncoder, AvifContainer, AvifEncoder, ChromaFormat,
};
use gamut_codec_abi::{EncodeConfig, Encoder, ImageDesc, Status};
use gamut_color::{
    BitDepth, ChromaSubsampling, ColorRange, MatrixCoefficients, Planar8, Planar16, RgbToYcbcr,
};
use gamut_core::{
    Dimensions, EncodeImage, Error, ErrorKind, Gray8, ImageRef, Result, Rgb8, Rgb16, Rgba8,
};

/// The fixture the golden files in `tests/data` were produced from: a 34×18 deterministic RGB ramp.
const W: u32 = 34;
const H: u32 = 18;

fn fixture() -> Vec<u8> {
    let mut rgb = vec![0u8; (W * H * 3) as usize];
    for (i, b) in rgb.iter_mut().enumerate() {
        *b = (i * 37) as u8;
    }
    rgb
}

fn dims() -> Dimensions {
    Dimensions {
        width: W,
        height: H,
    }
}

fn encode(encoder: &AvifEncoder) -> Result<Vec<u8>> {
    let rgb = fixture();
    let mut out = Vec::new();
    encoder.encode_image(ImageRef::<Rgb8>::new(&rgb, dims()).unwrap(), &mut out)?;
    Ok(out)
}

#[track_caller]
fn assert_owned_error(error: &Error, kind: ErrorKind, message: &'static str) {
    assert_eq!(error.kind(), kind);
    assert_eq!(error.static_message(), Some(message));
    assert_eq!(error.origin(), Some("gamut-avif"));
}

/// The AV1 OBU stream the built-in encoder produces for the fixture — a conformant stream a test
/// backend can hand back verbatim, so "the backend was used" is observable without a second AV1
/// encoder. `colour` must match the request's, or the crate rejects the stream for signalling a
/// different colour configuration than it asked for.
/// The chroma an encoder with this colour codes: lossless pins 4:4:4 (identity requires it), and
/// the lossy default is 4:2:0.
fn chroma_for(colour: Av1Colour) -> ChromaSubsampling {
    if colour == Av1Colour::default() {
        ChromaSubsampling::Cs444
    } else {
        ChromaSubsampling::Cs420
    }
}

fn builtin_obus(base_q_idx: u8, colour: Av1Colour, chroma: ChromaSubsampling) -> Vec<u8> {
    let planes = fixture_planes(colour, chroma);
    gamut_av1::encode_still_intra_with(&planes, base_q_idx, colour)
        .unwrap()
        .0
        .obus
}

/// The fixture in the plane layout `colour` describes: identity GBR, or YCbCr through its matrix.
fn fixture_planes(colour: Av1Colour, chroma: ChromaSubsampling) -> Planar8 {
    let rgb = fixture();
    match colour.matrix {
        MatrixCoefficients::Identity => Planar8::from_rgb8_identity(&rgb, W, H).unwrap(),
        matrix => {
            let img = ImageRef::<Rgb8>::new(&rgb, Dimensions::new(W, H).unwrap()).unwrap();
            let m = RgbToYcbcr::new(matrix, colour.range, BitDepth::Eight).unwrap();
            Planar8::from_rgb8_matrix_subsampled(img, m, chroma).unwrap()
        }
    }
}

// ================================================================================================
// The 1.0 additivity guarantee.
// ================================================================================================

/// **The 1.0 guarantee.** An `AvifEncoder` with no pushed backend must emit exactly the bytes the
/// built-in encoder produces — pushing a backend and then not using it must perturb nothing.
///
/// The goldens are re-captured whenever the built-in AV1 encoder deliberately changes what it
/// codes; they pin the seam's additivity, not the codec's output forever. Re-captured when
/// `gamut-av1` enabled CDF adaptation (`disable_cdf_update = 0`), which shrank these two files
/// from 4426/1864 bytes to 3407/1557 with an unchanged reconstruction; again when the lossy
/// encoder moved to BT.709 YCbCr, taking `lossy50` to 1323; and again when the lossy default
/// became 4:2:0, taking it to 763 — a 42% reduction at the same quality. The lossless golden is
/// unaffected by both colour changes: lossless stays on the identity matrix at 4:4:4, which AV1
/// §6.4.2 requires of it.
#[test]
fn default_encoder_output_is_byte_identical() {
    for (name, encoder) in [
        ("lossless", AvifEncoder::new()),
        ("lossy50", AvifEncoder::lossy(50)),
    ] {
        let golden = std::fs::read(format!(
            "{}/tests/data/default_{name}.avif",
            env!("CARGO_MANIFEST_DIR")
        ))
        .expect("golden fixture present");
        let got = encode(&encoder).expect("encodes");
        assert_eq!(
            got.len(),
            golden.len(),
            "{name}: output length changed ({} vs {} golden bytes)",
            got.len(),
            golden.len()
        );
        assert!(got == golden, "{name}: output bytes changed");
    }
    // `default()` and `lossless()` are the same encoder, so they share the lossless golden.
    assert_eq!(
        encode(&AvifEncoder::default()).unwrap(),
        encode(&AvifEncoder::lossless()).unwrap()
    );
}

/// The fixture's RGB channels with an added alpha ramp, so the colour planes a backend sees for an
/// `Rgba8` encode are byte-identical to the ones it sees for [`fixture`].
fn rgba_fixture() -> Vec<u8> {
    let rgb = fixture();
    let mut px = vec![0u8; (W * H * 4) as usize];
    for i in 0..(W * H) as usize {
        px[i * 4..i * 4 + 3].copy_from_slice(&rgb[i * 3..i * 3 + 3]);
        px[i * 4 + 3] = (i * 5) as u8;
    }
    px
}

#[test]
fn monochrome_jobs_never_reach_the_backend_registry() {
    // The seam's v1 contract is 8-bit 4:4:4 `seq_profile = 1`, and `Av1EncodeRequest` cannot
    // express anything else — so a backend written against it has no way to *decline* a monochrome
    // job. Offering one would hand it single-plane input it never agreed to encode, which is why
    // the alpha auxiliary and a `Gray8` primary go straight to the built-in tail. `Scripted`
    // asserts the 4:4:4 layout itself, so a monochrome job reaching it fails loudly rather than
    // silently.
    let log = Arc::new(Mutex::new(Vec::new()));
    let mut encoder = AvifEncoder::new();
    encoder.push_backend(Scripted::new("b", true, Outcome::Passthrough, &log));

    // `Gray8` is monochrome end to end: the registry is not consulted at all.
    let gray: Vec<u8> = (0..(W * H)).map(|i| (i * 37) as u8).collect();
    encoder
        .encode_to_vec(ImageRef::<Gray8>::new(&gray, dims()).unwrap())
        .expect("grayscale encodes through the built-in tail");
    assert!(log.lock().unwrap().is_empty(), "no backend call for Gray8");

    // `Rgba8` splits: the 4:4:4 colour item is a job the backend owns, its monochrome alpha
    // auxiliary is not. Exactly one supports/encode pair, for the colour half.
    let rgba = rgba_fixture();
    encoder
        .encode_to_vec(ImageRef::<Rgba8>::new(&rgba, dims()).unwrap())
        .expect("rgba encodes");
    assert_eq!(*log.lock().unwrap(), ["b:supports", "b:encode"]);
}

/// The fixture on the canonical full 16-bit scale (each byte replicated, `v * 0x101`), so
/// narrowing it back to 10 or 12 bits is a value the test can predict.
fn fixture16() -> Vec<u16> {
    fixture().iter().map(|&b| u16::from(b) * 0x101).collect()
}

fn encode16(encoder: &AvifEncoder) -> Result<Vec<u8>> {
    let px = fixture16();
    encoder.encode_to_vec(ImageRef::<Rgb16>::new(&px, dims()).unwrap())
}

/// A backend that opts into high bit depth by overriding `encode_still16`, and hands back the
/// built-in encoder's own stream for the request it was given.
struct Wide {
    log: Arc<Mutex<Vec<String>>>,
    depth_seen: Arc<Mutex<Option<BitDepth>>>,
}

impl Av1StillEncoder for Wide {
    fn supports(&mut self, _req: &Av1EncodeRequest) -> bool {
        true
    }

    fn encode_still(&mut self, _req: &Av1EncodeRequest, _planes: &Planar8) -> Result<Vec<u8>> {
        panic!("the 8-bit entry point must not be used for a 16-bit job")
    }

    fn encode_still16(&mut self, req: &Av1EncodeRequest, planes: &Planar16) -> Result<Vec<u8>> {
        self.log.lock().unwrap().push("wide:encode16".into());
        *self.depth_seen.lock().unwrap() = Some(req.bit_depth());
        assert_eq!(planes.bit_depth(), req.bit_depth());
        Ok(
            gamut_av1::encode_still_intra16_with(planes, req.base_q_idx(), req.colour())
                .expect("the built-in encoder handles the request")
                .0
                .obus,
        )
    }
}

/// A backend that accepts every job but never overrode `encode_still16`: the **default** must
/// decline, so a high-bit-depth job falls through to the built-in tail instead of being handed
/// samples the backend's 8-bit contract never covered.
#[test]
fn a_backend_without_encode_still16_declines_high_bit_depth() {
    let log = log();
    let mut encoder = AvifEncoder::new();
    // `Outcome::Fail` would be a terminal error if the 8-bit entry point were reached, so this also
    // pins that it is not.
    encoder.push_backend(Scripted::new(
        "b",
        true,
        Outcome::Fail("the 8-bit entry point must not be used"),
        &log,
    ));
    let with_backend = encode16(&encoder).expect("falls through to the built-in tail");
    assert_eq!(events(&log), ["b:supports"]);
    assert_eq!(
        with_backend,
        encode16(&AvifEncoder::new()).unwrap(),
        "the fall-through output is the backend-free output"
    );
}

#[test]
fn a_backend_that_opts_in_owns_the_high_bit_depth_job() {
    let log = log();
    let depth_seen = Arc::new(Mutex::new(None));
    let mut encoder = AvifEncoder::new();
    encoder.push_backend(Wide {
        log: Arc::clone(&log),
        depth_seen: Arc::clone(&depth_seen),
    });
    let out = encode16(&encoder).expect("the backend owns the job");
    assert_eq!(events(&log), ["wide:encode16"]);
    // The request states the depth, which is how a backend decides what to emit.
    assert_eq!(*depth_seen.lock().unwrap(), Some(BitDepth::Twelve));
    // The container re-derives `av1C`/`colr` from the returned stream, so a backend-supplied
    // 12-bit picture is described exactly as the built-in one is.
    assert_eq!(out, encode16(&AvifEncoder::new()).unwrap());
}

#[test]
fn a_backend_stream_at_the_wrong_depth_is_rejected() {
    // The container stamps `av1C` and `pixi` from the request's depth, so a stream coded at another
    // one would be published under a description it does not match.
    struct TenBit;
    impl Av1StillEncoder for TenBit {
        fn supports(&mut self, _req: &Av1EncodeRequest) -> bool {
            true
        }
        fn encode_still(&mut self, _req: &Av1EncodeRequest, _planes: &Planar8) -> Result<Vec<u8>> {
            unreachable!("16-bit job")
        }
        fn encode_still16(&mut self, req: &Av1EncodeRequest, planes: &Planar16) -> Result<Vec<u8>> {
            // Re-narrow the 12-bit planes to 10 and code those instead.
            let ten = Planar16::from_planes(
                planes.width(),
                planes.height(),
                BitDepth::Ten,
                std::array::from_fn(|i| planes.plane(i).iter().map(|&s| s >> 2).collect()),
            )?;
            Ok(
                gamut_av1::encode_still_intra16_with(&ten, req.base_q_idx(), req.colour())?
                    .0
                    .obus,
            )
        }
    }
    let mut encoder = AvifEncoder::new();
    encoder.push_backend(TenBit);
    let err = encode16(&encoder).expect_err("a depth mismatch is rejected");
    assert_owned_error(
        &err,
        ErrorKind::InvalidInput,
        "AVIF: AV1 backend stream is coded at a different bit depth than requested",
    );
}

// ================================================================================================
// Test backends.
// ================================================================================================

/// A scriptable backend: records every request it is asked about, answers `supports` from
/// `accepts`, and then returns `result`.
struct Scripted {
    /// Whether `supports` accepts.
    accepts: bool,
    /// What `encode_still` returns once accepted.
    result: Outcome,
    /// Shared log of `(label, phase)` events, in call order.
    log: Arc<Mutex<Vec<String>>>,
    /// This backend's label in the log.
    label: &'static str,
}

/// What a scripted backend does once it has accepted a job.
#[derive(Clone)]
enum Outcome {
    /// Return the built-in encoder's OBUs for the request's `base_q_idx` (a conformant stream).
    Passthrough,
    /// Return the given bytes verbatim (used for malformed-stream cases).
    Bytes(Vec<u8>),
    /// Fail with `Error::Unsupported(msg)`.
    Fail(&'static str),
}

impl Scripted {
    fn new(
        label: &'static str,
        accepts: bool,
        result: Outcome,
        log: &Arc<Mutex<Vec<String>>>,
    ) -> Self {
        Self {
            accepts,
            result,
            log: Arc::clone(log),
            label,
        }
    }

    fn record(&self, phase: &str) {
        self.log
            .lock()
            .unwrap()
            .push(format!("{}:{phase}", self.label));
    }
}

impl Av1StillEncoder for Scripted {
    fn supports(&mut self, req: &Av1EncodeRequest) -> bool {
        // The request must describe the job exactly: display dimensions and the *derived*
        // quantizer, never the 0..=100 quality scale.
        assert_eq!(req.dimensions(), dims());
        assert_eq!(req.width(), W);
        assert_eq!(req.height(), H);
        assert_eq!(req.is_lossless(), req.base_q_idx() == 0);
        self.record("supports");
        self.accepts
    }

    fn encode_still(&mut self, req: &Av1EncodeRequest, planes: &Planar8) -> Result<Vec<u8>> {
        assert_eq!((planes.width(), planes.height()), (W, H));
        // The planes must be in exactly the layout the request's colour describes — identity GBR
        // for the lossless job, YCbCr through the matrix for the lossy one.
        // The planes must match the request's chroma as well as its colour: the request states
        // both, and a backend that ignored either would be handed samples it could not describe.
        assert_eq!(planes.subsampling(), req.chroma());
        let expected = fixture_planes(req.colour(), req.chroma());
        for p in 0..3 {
            assert_eq!(planes.plane(p), expected.plane(p), "plane {p}");
        }
        self.record("encode");
        match &self.result {
            Outcome::Passthrough => Ok(builtin_obus(req.base_q_idx(), req.colour(), req.chroma())),
            Outcome::Bytes(b) => Ok(b.clone()),
            Outcome::Fail(msg) => Err(Error::Unsupported(msg)),
        }
    }
}

fn log() -> Arc<Mutex<Vec<String>>> {
    Arc::new(Mutex::new(Vec::new()))
}

fn events(log: &Arc<Mutex<Vec<String>>>) -> Vec<String> {
    log.lock().unwrap().clone()
}

// ================================================================================================
// The fallback contract.
// ================================================================================================

/// Push order: the first backend that supports the job wins, later backends are never consulted,
/// and its bytes — not the built-in encoder's — reach the container.
#[test]
fn first_supporting_backend_wins_in_push_order() {
    let log = log();
    let mut encoder = AvifEncoder::lossy(50);
    encoder
        .push_backend(Scripted::new("a", true, Outcome::Passthrough, &log))
        .push_backend(Scripted::new("b", true, Outcome::Passthrough, &log));
    let out = encode(&encoder).expect("first backend encodes");
    assert_eq!(
        events(&log),
        vec!["a:supports".to_string(), "a:encode".to_string()],
        "only the first backend is consulted"
    );
    // The passthrough backend returns exactly the built-in stream, so the file is the golden one —
    // proving the backend's bytes were used as-is and the container was rebuilt around them.
    assert_eq!(out, encode(&AvifEncoder::lossy(50)).unwrap());
}

/// A backend that declines is skipped; the next one is tried.
#[test]
fn declining_backends_are_skipped() {
    let log = log();
    let mut encoder = AvifEncoder::lossy(50);
    encoder
        .push_backend(Scripted::new("no", false, Outcome::Passthrough, &log))
        .push_backend(Scripted::new("yes", true, Outcome::Passthrough, &log));
    encode(&encoder).expect("second backend encodes");
    assert_eq!(
        events(&log),
        vec![
            "no:supports".to_string(),
            "yes:supports".to_string(),
            "yes:encode".to_string()
        ]
    );
}

/// When every backend declines, the built-in `gamut-av1` tail runs — and the output is the
/// no-backend output, byte for byte.
#[test]
fn all_declining_falls_through_to_the_builtin_tail() {
    let log = log();
    let mut encoder = AvifEncoder::new();
    encoder
        .push_backend(Scripted::new("a", false, Outcome::Passthrough, &log))
        .push_backend(Scripted::new("b", false, Outcome::Passthrough, &log));
    let out = encode(&encoder).expect("built-in tail encodes");
    assert_eq!(
        events(&log),
        vec!["a:supports".to_string(), "b:supports".to_string()],
        "no backend encodes"
    );
    assert_eq!(out, encode(&AvifEncoder::new()).unwrap(), "tail output");
}

/// Accepted-then-failed propagates: the error surfaces unchanged and the built-in tail is **not**
/// used (a silent fallback would make the output non-deterministic).
#[test]
fn accepted_then_failed_propagates_and_skips_the_tail() {
    let log = log();
    let mut encoder = AvifEncoder::new();
    encoder
        .push_backend(Scripted::new(
            "boom",
            true,
            Outcome::Fail("backend exploded"),
            &log,
        ))
        .push_backend(Scripted::new("later", true, Outcome::Passthrough, &log));
    let err = encode(&encoder).expect_err("the accepted backend's error propagates");
    assert!(
        matches!(err, Error::Unsupported("backend exploded")),
        "unexpected error: {err:?}"
    );
    assert_eq!(
        events(&log),
        vec!["boom:supports".to_string(), "boom:encode".to_string()],
        "no later backend and no tail after an accepted failure"
    );
}

/// `Clone` shares backends: a clone drives the *same* backend objects, and a backend pushed after
/// the clone was taken belongs to the original only.
#[test]
fn clone_shares_backends() {
    let log = log();
    let mut encoder = AvifEncoder::lossy(50);
    encoder.push_backend(Scripted::new("shared", true, Outcome::Passthrough, &log));
    let clone = encoder.clone();

    encode(&clone).expect("the clone sees the pushed backend");
    assert_eq!(
        events(&log),
        vec!["shared:supports".to_string(), "shared:encode".to_string()],
        "the clone drove the shared backend"
    );

    // A push after cloning affects only the original's registry.
    encoder.push_backend(Scripted::new("late", true, Outcome::Passthrough, &log));
    log.lock().unwrap().clear();
    encode(&clone).expect("clone still encodes");
    assert_eq!(
        events(&log),
        vec!["shared:supports".to_string(), "shared:encode".to_string()],
        "the clone's registry did not gain the later backend"
    );
}

/// A backend that panics poisons its mutex; every later encode reports that as a typed error
/// rather than using a half-updated backend (or the tail, which would hide the failure).
#[test]
fn a_panicking_backend_poisons_the_registry() {
    /// Panics from `supports` on its first call.
    struct Panicky;
    impl Av1StillEncoder for Panicky {
        fn supports(&mut self, _req: &Av1EncodeRequest) -> bool {
            panic!("backend panicked");
        }
        fn encode_still(&mut self, _r: &Av1EncodeRequest, _p: &Planar8) -> Result<Vec<u8>> {
            unreachable!("never accepted")
        }
    }
    let mut encoder = AvifEncoder::new();
    encoder.push_backend(Panicky);
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let first = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| encode(&encoder)));
    std::panic::set_hook(hook);
    assert!(first.is_err(), "the backend's panic unwinds to the caller");

    let err = encode(&encoder).expect_err("the poisoned registry is reported");
    assert_owned_error(
        &err,
        ErrorKind::InvalidInput,
        "AVIF: AV1 encode backend is poisoned",
    );
}

/// `Debug` survives the (non-`Debug`) registry, summarizing it by count.
#[test]
fn debug_reports_the_backend_count() {
    let log = log();
    let mut encoder = AvifEncoder::new();
    assert!(
        format!("{encoder:?}").contains("backends: 0"),
        "{encoder:?}"
    );
    encoder.push_backend(Scripted::new("a", false, Outcome::Passthrough, &log));
    let rendered = format!("{encoder:?}");
    assert!(rendered.contains("backends: 1"), "{rendered}");
    assert!(rendered.contains("AvifEncoder"), "{rendered}");
    assert!(rendered.contains("Lossless"), "{rendered}");
}

// ================================================================================================
// Validation of a backend-supplied stream.
// ================================================================================================

/// A stream whose coded dimensions disagree with the image is rejected rather than stamped into a
/// container whose `ispe` would then lie.
#[test]
fn backend_stream_dimensions_must_match() {
    let other = Planar8::from_rgb8_identity(&vec![7u8; 16 * 8 * 3], 16, 8).unwrap();
    let wrong = gamut_av1::encode_still_intra(&other, 0).unwrap().0.obus;
    let log = log();
    let mut encoder = AvifEncoder::new();
    encoder.push_backend(Scripted::new("wrong", true, Outcome::Bytes(wrong), &log));
    let err = encode(&encoder).expect_err("dimension mismatch rejected");
    assert_owned_error(
        &err,
        ErrorKind::InvalidInput,
        "AVIF: AV1 backend stream dimensions differ from the image",
    );
}

/// A stream with no sequence header OBU is rejected.
#[test]
fn backend_stream_without_sequence_header_is_rejected() {
    let log = log();
    let mut encoder = AvifEncoder::new();
    // A lone temporal-delimiter OBU: well-formed OBU framing, no sequence header.
    encoder.push_backend(Scripted::new(
        "hdrless",
        true,
        Outcome::Bytes(vec![0x12, 0x00]),
        &log,
    ));
    let err = encode(&encoder).expect_err("missing sequence header rejected");
    assert_owned_error(
        &err,
        ErrorKind::InvalidInput,
        "AVIF: AV1 backend stream has no sequence header OBU",
    );
}

/// A sequence header without `reduced_still_picture_header` is out of the v1 surface's scope.
#[test]
fn backend_stream_must_be_a_reduced_still_picture() {
    // seq header OBU payload byte 0: seq_profile(3)=1 | still_picture(1)=1 | reduced(1)=0 | …
    let payload = [0b0011_0000u8, 0x00, 0x00, 0x00];
    let mut obus = vec![0x0A, payload.len() as u8];
    obus.extend_from_slice(&payload);
    let log = log();
    let mut encoder = AvifEncoder::new();
    encoder.push_backend(Scripted::new("full", true, Outcome::Bytes(obus), &log));
    let err = encode(&encoder).expect_err("non-reduced sequence header rejected");
    assert_owned_error(
        &err,
        ErrorKind::Unsupported,
        "AVIF: AV1 backend stream must set reduced_still_picture_header",
    );
}

/// §6.4.1 defines `seq_profile` 0, 1 and 2 and reserves 3..=7. `color_config()`'s own layout
/// depends on the profile, so a stream this parser cannot describe is refused before its colour
/// fields are read.
#[test]
fn backend_stream_must_use_a_defined_profile() {
    // seq_profile(3)=3 | still_picture=1 | reduced=1 | seq_level_idx[0](5)=0 | width/height bits.
    // 0b011_1_1_000 = 0x78.
    let payload = [0x78u8, 0b0001_0101, 0b0100_0000, 0x00, 0x00, 0x00];
    let mut obus = vec![0x0A, payload.len() as u8];
    obus.extend_from_slice(&payload);
    let log = log();
    let mut encoder = AvifEncoder::new();
    encoder.push_backend(Scripted::new("p3", true, Outcome::Bytes(obus), &log));
    let err = encode(&encoder).expect_err("a reserved profile is rejected");
    assert_owned_error(
        &err,
        ErrorKind::Unsupported,
        "AVIF: AV1 backend stream must use seq_profile 0, 1 or 2",
    );
}

/// Profiles 0 and 2 are parsed — they are the monochrome and 12-bit profiles — but the item being
/// built here is the three-plane *colour* item, so a `mono_chrome = 1` stream cannot describe it.
#[test]
fn backend_stream_must_be_three_plane() {
    // A real monochrome stream rather than a hand-built stub, so the rejection is reached through
    // a genuine profile-0 `color_config()` and not through a malformed header.
    let luma: Vec<u8> = (0..(W * H)).map(|i| (i * 37) as u8).collect();
    let mono = Planar8::from_planes_subsampled(
        W,
        H,
        gamut_color::ChromaSubsampling::Cs400,
        [luma, Vec::new(), Vec::new()],
    )
    .unwrap();
    let obus = gamut_av1::encode_still_intra_with(&mono, 0, Av1Colour::monochrome())
        .unwrap()
        .0
        .obus;
    let log = log();
    let mut encoder = AvifEncoder::new();
    encoder.push_backend(Scripted::new("mono", true, Outcome::Bytes(obus), &log));
    let err = encode(&encoder).expect_err("a monochrome stream is rejected for a colour item");
    assert_owned_error(
        &err,
        ErrorKind::Unsupported,
        "AVIF: AV1 backend stream must be three-plane (mono_chrome = 0)",
    );
}

/// §5.5.2 *derives* a stream's subsampling from `seq_profile` rather than coding it, so a
/// backend-supplied 4:2:0 or 4:2:2 stream carries no bit saying so. The rebuilt `av1C` must
/// nonetheless **describe** it: AV1-ISOBMFF v1.3.0 §2.3.4 makes the record a mirror of the
/// sequence header, and a record that stamped 4:4:4 regardless would publish a chroma format the
/// payload does not have.
///
/// This replaces `backend_stream_must_be_four_four_four`, which asserted the opposite — that any
/// subsampled backend stream was *refused*. That was the right contract only while the crate could
/// neither emit nor describe a subsampled stream; 4:2:0 is now the default lossy output, so
/// refusing one would reject exactly the stream a backend is asked to produce. The obligation the
/// old test was protecting (the record must not lie) is what is asserted here instead.
#[test]
fn a_subsampled_backend_stream_is_described_by_its_av1c_record() {
    let colour = Av1Colour {
        matrix: MatrixCoefficients::Bt709,
        ..Av1Colour::default()
    };
    // 4:2:0 — AV1 Main, and the default lossy request.
    let log420 = log();
    let mut encoder = AvifEncoder::lossy(50);
    encoder.push_backend(Scripted::new(
        "p0-420",
        true,
        Outcome::Bytes(builtin_obus(127, colour, ChromaSubsampling::Cs420)),
        &log420,
    ));
    let file = encode(&encoder).expect("a 4:2:0 stream answering a 4:2:0 request is accepted");
    let container = AvifContainer::parse(&file).expect("the emitted file parses");
    let record = container
        .image()
        .primary_item()
        .av1_config()
        .expect("av1C present")
        .expect("av1C parses");
    assert_eq!(record.seq_profile, 0, "4:2:0 is AV1 Main");
    assert_eq!(
        (record.chroma_subsampling_x, record.chroma_subsampling_y),
        (1, 1)
    );
    assert_eq!(record.chroma_format(), ChromaFormat::Yuv420);
    assert!(!record.monochrome && !record.high_bitdepth);

    // 4:2:2 — AV1 Professional, and subsampled on one axis only, so a record that carried a single
    // "is subsampled" bit could not tell the two apart.
    let log422 = log();
    let mut encoder = AvifEncoder::lossy(50).with_chroma(ChromaSubsampling::Cs422);
    encoder.push_backend(Scripted::new(
        "p2-422",
        true,
        Outcome::Bytes(builtin_obus(127, colour, ChromaSubsampling::Cs422)),
        &log422,
    ));
    let file = encode(&encoder).expect("a 4:2:2 stream answering a 4:2:2 request is accepted");
    let container = AvifContainer::parse(&file).expect("the emitted file parses");
    let record = container
        .image()
        .primary_item()
        .av1_config()
        .expect("av1C present")
        .expect("av1C parses");
    assert_eq!(record.seq_profile, 2, "4:2:2 is AV1 Professional");
    assert_eq!(
        (record.chroma_subsampling_x, record.chroma_subsampling_y),
        (1, 0)
    );
    assert_eq!(record.chroma_format(), ChromaFormat::Yuv422);
}

#[test]
fn backend_stream_must_signal_the_requested_chroma() {
    // The default lossy encoder asks for 4:2:0 (profile 0); hand back an otherwise-conformant
    // 4:4:4 stream. `av1C` mirrors the sequence header, so accepting this would publish a chroma
    // format that disagrees with the payload (AV1-ISOBMFF §2.3.4).
    let colour = Av1Colour {
        matrix: MatrixCoefficients::Bt709,
        ..Av1Colour::default()
    };
    let stream = builtin_obus(127, colour, ChromaSubsampling::Cs444);
    let log = log();
    let mut encoder = AvifEncoder::lossy(50);
    encoder.push_backend(Scripted::new(
        "wrong-chroma",
        true,
        Outcome::Bytes(stream),
        &log,
    ));
    let err = encode(&encoder).expect_err("4:4:4 stream for a 4:2:0 request is rejected");
    assert_owned_error(
        &err,
        ErrorKind::InvalidInput,
        "AVIF: AV1 backend stream signals a different chroma format than requested",
    );
}

#[test]
fn backend_stream_shortcut_must_not_contradict_its_profile() {
    // The §5.5.2 sRGB shortcut infers 4:4:4 whatever the profile declares, so a profile-0 stream
    // carrying BT.709 + sRGB + identity asserts two chroma formats at once. libaom asserts against
    // exactly this construction, so no conformant encoder produces one.
    //
    // seq_profile(3)=0 | still_picture=1 | reduced=1 | seq_level_idx[0](5)=0, then the width/height
    // bit counts and dimensions, six enable flags, high_bitdepth=0, mono_chrome=0,
    // color_description_present=1, and cp=1 / tc=13 / mc=0.
    let mut w = BitVec::default();
    w.push_bits(0, 3); // seq_profile
    w.push_bits(1, 1); // still_picture
    w.push_bits(1, 1); // reduced_still_picture_header
    w.push_bits(0, 5); // seq_level_idx[0]
    w.push_bits(5, 4); // frame_width_bits_minus_1
    w.push_bits(4, 4); // frame_height_bits_minus_1
    w.push_bits(W - 1, 6);
    w.push_bits(H - 1, 5);
    w.push_bits(0, 6); // the six enable flags
    w.push_bits(0, 1); // high_bitdepth
    w.push_bits(0, 1); // mono_chrome (coded because the profile is not High)
    w.push_bits(1, 1); // color_description_present_flag
    w.push_bits(1, 8); // color_primaries = BT.709
    w.push_bits(13, 8); // transfer_characteristics = sRGB
    w.push_bits(0, 8); // matrix_coefficients = identity
    let payload = w.finish();
    let mut obus = vec![0x0A, payload.len() as u8];
    obus.extend_from_slice(&payload);
    let log = log();
    let mut encoder = AvifEncoder::lossy(50);
    encoder.push_backend(Scripted::new(
        "shortcut-p0",
        true,
        Outcome::Bytes(obus),
        &log,
    ));
    let err = encode(&encoder).expect_err("shortcut on profile 0 is contradictory");
    assert_owned_error(
        &err,
        ErrorKind::InvalidInput,
        "AVIF: AV1 backend stream takes the sRGB color_config shortcut, which infers 4:4:4, but \
         its seq_profile declares subsampled chroma",
    );
}

/// A minimal MSB-first bit writer, for hand-assembling sequence-header payloads.
#[derive(Default)]
struct BitVec {
    bytes: Vec<u8>,
    bit: u32,
}

impl BitVec {
    fn push_bits(&mut self, value: u32, n: u32) {
        for i in (0..n).rev() {
            if self.bit == 0 {
                self.bytes.push(0);
            }
            let b = ((value >> i) & 1) as u8;
            let last = self.bytes.len() - 1;
            self.bytes[last] |= b << (7 - self.bit);
            self.bit = (self.bit + 1) % 8;
        }
    }

    fn finish(self) -> Vec<u8> {
        self.bytes
    }
}

/// A backend stream whose `color_config()` disagrees with the request is rejected: the container
/// mirrors the sequence header into `colr`, so accepting it would publish a colour description the
/// samples do not have.
#[test]
fn backend_stream_must_signal_the_requested_colour() {
    // The default lossy encoder asks for BT.709; hand back an otherwise-conformant stream that
    // signals identity instead.
    let identity_stream = builtin_obus(127, Av1Colour::default(), ChromaSubsampling::Cs444);
    let log = log();
    let mut encoder = AvifEncoder::lossy(50);
    encoder.push_backend(Scripted::new(
        "wrong-colour",
        true,
        Outcome::Bytes(identity_stream),
        &log,
    ));
    let err = encode(&encoder).expect_err("mismatched colour rejected");
    assert_owned_error(
        &err,
        ErrorKind::InvalidInput,
        "AVIF: AV1 backend stream signals a different colour configuration than requested",
    );

    // …and a matching stream is accepted, so the check is not simply refusing every backend.
    //
    // The three encoders below make the parser's AV1 §5.5.2 branch load-bearing in both
    // directions. `lossless()` signals BT.709 + sRGB + identity, which **takes** the shortcut: no
    // `color_range` bit is coded, so a parser that read one would consume `separate_uv_delta_q`
    // (0) and report studio range. `lossy(50)` misses the shortcut on the matrix alone, and the
    // studio-range encoder misses it *and* codes `color_range = 0` — so a parser that skipped the
    // bit would report full range. Either way the mismatch check fires and the encode fails.
    for (name, encoder) in [
        ("shortcut", AvifEncoder::lossless()),
        ("bt709-full", AvifEncoder::lossy(50)),
        (
            "bt709-studio",
            AvifEncoder::lossy(50).with_color_range(ColorRange::Limited),
        ),
    ] {
        let colour = encoder_colour(&encoder);
        let base_q_idx = if colour == Av1Colour::default() {
            0
        } else {
            127
        };
        let mut with_backend = encoder.clone();
        with_backend.push_backend(Scripted::new(
            "right-colour",
            true,
            Outcome::Bytes(builtin_obus(base_q_idx, colour, chroma_for(colour))),
            &log,
        ));
        assert_eq!(
            encode(&with_backend)
                .unwrap_or_else(|e| panic!("{name}: matching colour rejected: {e}")),
            encode(&encoder).unwrap(),
            "{name}"
        );
    }
}

/// The colour an encoder's configuration selects, mirroring `AvifEncoder::colour` — the test needs
/// it to build a stream the crate will accept.
fn encoder_colour(encoder: &AvifEncoder) -> Av1Colour {
    let config = encoder.config();
    match config.mode {
        gamut_avif::AvifMode::Lossless => Av1Colour::default(),
        _ => Av1Colour {
            matrix: config.matrix,
            range: config.range,
            ..Av1Colour::default()
        },
    }
}

/// A stream that leaves its CICP code points UNSPECIFIED is rejected with a message that says so,
/// rather than falling out of the colour comparison as a confusing mismatch.
#[test]
fn backend_stream_must_describe_its_colour() {
    // A hand-built reduced-still-picture sequence header for the 34×18 fixture with
    // `color_description_present_flag = 0`:
    //   seq_profile(3)=1 | still_picture=1 | reduced=1 | seq_level_idx[0](5)=0
    //   | frame_width_bits_minus_1(4)=5 | frame_height_bits_minus_1(4)=4
    //   | max_frame_width_minus_1(6)=33 | max_frame_height_minus_1(5)=17
    //   | use_128x128_superblock, filter_intra, intra_edge_filter, superres, cdef, restoration = 0
    //   | high_bitdepth=0 | color_description_present_flag=0 | color_range=0
    //   | separate_uv_delta_q=0 | film_grain_params_present=0 | trailing one + zero pad
    let payload = [0x38u8, 0x15, 0x21, 0x88, 0x00, 0x80];
    let mut obus = vec![0x0A, payload.len() as u8];
    obus.extend_from_slice(&payload);
    let log = log();
    let mut encoder = AvifEncoder::lossy(50);
    encoder.push_backend(Scripted::new("no-cicp", true, Outcome::Bytes(obus), &log));
    let err = encode(&encoder).expect_err("undescribed colour rejected");
    assert_owned_error(
        &err,
        ErrorKind::Unsupported,
        "AVIF: AV1 backend stream must set color_description_present_flag",
    );
}

/// A truncated sequence header (too few bits for the dimension fields) is reported, not panicked on.
#[test]
fn truncated_sequence_header_is_reported() {
    // seq_profile=1, still_picture=1, reduced=1, then the payload simply ends.
    let payload = [0b0011_1000u8];
    let mut obus = vec![0x0A, payload.len() as u8];
    obus.extend_from_slice(&payload);
    let log = log();
    let mut encoder = AvifEncoder::new();
    encoder.push_backend(Scripted::new("short", true, Outcome::Bytes(obus), &log));
    let err = encode(&encoder).expect_err("truncated header rejected");
    assert_owned_error(
        &err,
        ErrorKind::InvalidInput,
        "AVIF: AV1 backend sequence header truncated",
    );
}

// ================================================================================================
// The codec-abi adapter.
// ================================================================================================

/// The `ImageDesc` fields a test asserts on: pixel format, dimensions, depth, plane count,
/// strides, and the first sample of each of the three planes.
type SeenImage = (u32, u32, u32, u32, u32, [usize; 4], [u8; 3]);

/// A scriptable `gamut_codec_abi::Encoder`, capturing what the adapter lowered into the descriptors.
struct AbiStub {
    /// `supports` answer.
    supports: Status,
    /// `encode` answer.
    encode: Status,
    /// Chunks the stub streams through the sink before returning.
    chunks: Vec<Vec<u8>>,
    /// The `(codec_id, quality, base_q_idx)` the adapter passed, captured at `encode` time.
    seen_config: Option<(u32, u32, Option<u8>)>,
    /// The image descriptor the adapter lowered, captured at `encode` time.
    seen_image: Option<SeenImage>,
}

impl AbiStub {
    fn new(supports: Status, encode: Status, chunks: Vec<Vec<u8>>) -> Self {
        Self {
            supports,
            encode,
            chunks,
            seen_config: None,
            seen_image: None,
        }
    }

    /// Reads back the `(codec_id, quality, base_q_idx-from-extra)` triple from a lowered config.
    fn capture_config(cfg: &EncodeConfig) -> (u32, u32, Option<u8>) {
        assert!(cfg.is_abi_current(), "struct_size guard filled in");
        let q_idx = (cfg.extra_len == 1 && !cfg.extra.is_null()).then(|| {
            // SAFETY-equivalent: the adapter documents `extra` as one byte, the AV1 base_q_idx,
            // borrowed for the call.
            unsafe { *cfg.extra.cast::<u8>() }
        });
        (cfg.codec_id, cfg.quality, q_idx)
    }
}

impl Encoder for AbiStub {
    fn supports(&mut self, cfg: &EncodeConfig) -> bool {
        self.seen_config = Some(Self::capture_config(cfg));
        self.supports.is_ok()
    }

    fn encode(
        &mut self,
        cfg: &EncodeConfig,
        image: &ImageDesc,
        sink: &mut dyn FnMut(&[u8]) -> Status,
    ) -> Status {
        self.seen_config = Some(Self::capture_config(cfg));
        assert!(image.is_abi_current());
        let first = [0, 1, 2].map(|i| unsafe { *image.planes[i] });
        self.seen_image = Some((
            image.pixel_format,
            image.width,
            image.height,
            image.depth,
            image.plane_count,
            image.strides,
            first,
        ));
        if !self.encode.is_ok() {
            return self.encode;
        }
        for chunk in &self.chunks {
            let status = sink(chunk);
            if !status.is_ok() {
                return status;
            }
        }
        Status::OK
    }
}

/// A shared handle so a test can inspect the stub after the adapter has consumed it.
#[derive(Clone)]
struct SharedStub(Arc<Mutex<AbiStub>>);

impl Encoder for SharedStub {
    fn supports(&mut self, cfg: &EncodeConfig) -> bool {
        self.0.lock().unwrap().supports(cfg)
    }

    fn encode(
        &mut self,
        cfg: &EncodeConfig,
        image: &ImageDesc,
        sink: &mut dyn FnMut(&[u8]) -> Status,
    ) -> Status {
        self.0.lock().unwrap().encode(cfg, image, sink)
    }
}

/// The adapter's happy path: the sink's chunks become the OBU payload, and the descriptors carry
/// the AV1 codec id, the `base_q_idx` in `extra` (never a `0..=100` quality), and the three
/// planes.
#[test]
fn abi_adapter_collects_sink_bytes_and_lowers_the_descriptors() {
    // The encoder under test is `lossy(50)`, whose colour is the BT.709 default.
    let colour = Av1Colour {
        matrix: MatrixCoefficients::Bt709,
        ..Av1Colour::default()
    };
    let obus = builtin_obus(127, colour, ChromaSubsampling::Cs420);
    // Two chunks, so the adapter's accumulation (not just a single hand-off) is exercised.
    let (head, tail) = obus.split_at(7);
    let stub = SharedStub(Arc::new(Mutex::new(AbiStub::new(
        Status::OK,
        Status::OK,
        vec![head.to_vec(), tail.to_vec()],
    ))));
    let mut encoder = AvifEncoder::lossy(50); // quality 50 ⇒ base_q_idx 127
    encoder.push_backend(AbiAv1StillEncoder::new(stub.clone()));
    let out = encode(&encoder).expect("adapter encodes");
    assert_eq!(
        out,
        encode(&AvifEncoder::lossy(50)).unwrap(),
        "the sink bytes became the item payload"
    );

    let seen = stub.0.lock().unwrap();
    assert_eq!(
        seen.seen_config,
        Some((u32::from_be_bytes(*b"av01"), 0, Some(127))),
        "codec id av01, quality unused (0), base_q_idx in extra"
    );
    // The lowered plane pointers are the encoder's own BT.709 planes, not the raw RGB — the
    // adapter must hand a backend exactly what the request's colour describes.
    let planes = fixture_planes(colour, ChromaSubsampling::Cs420);
    let first = [planes.plane(0)[0], planes.plane(1)[0], planes.plane(2)[0]];
    assert_ne!(
        first,
        [fixture()[1], fixture()[2], fixture()[0]],
        "BT.709 planes must differ from the identity GBR mapping"
    );
    assert_eq!(
        seen.seen_image,
        Some((
            gamut_core::PixelFormat::Rgb8 as u32,
            W,
            H,
            8,
            3,
            // Per-plane strides: luma at full width, chroma halved by the 4:2:0 request. A single
            // luma stride for all three would hand a backend a chroma plane it cannot address.
            [
                W as usize,
                W.div_ceil(2) as usize,
                W.div_ceil(2) as usize,
                0,
            ],
            first,
        ))
    );
}

/// `AV1_CODEC_ID` is the `av01` FourCC and is what the adapter stamps.
#[test]
fn av1_codec_id_is_the_av01_fourcc() {
    assert_eq!(gamut_avif::AV1_CODEC_ID, u32::from_be_bytes(*b"av01"));
    assert_eq!(gamut_avif::AV1_CODEC_ID, 0x6176_3031);
}

/// `Status::UNSUPPORTED` from `supports` declines, so the built-in tail runs.
#[test]
fn abi_adapter_unsupported_declines() {
    let stub = SharedStub(Arc::new(Mutex::new(AbiStub::new(
        Status::UNSUPPORTED,
        Status::OK,
        vec![],
    ))));
    let mut encoder = AvifEncoder::new();
    encoder.push_backend(AbiAv1StillEncoder::new(stub.clone()));
    let out = encode(&encoder).expect("falls through to the built-in tail");
    assert_eq!(out, encode(&AvifEncoder::new()).unwrap());
    assert!(
        stub.0.lock().unwrap().seen_image.is_none(),
        "a declining backend is never asked to encode"
    );
}

/// A **late** `UNSUPPORTED` — accepted at `supports`, declined at `encode` — also falls through:
/// to the next backend first, and to the built-in tail when there is none.
#[test]
fn abi_adapter_late_unsupported_falls_through() {
    let late = SharedStub(Arc::new(Mutex::new(AbiStub::new(
        Status::OK,
        Status::UNSUPPORTED,
        vec![],
    ))));
    let log = log();
    let mut encoder = AvifEncoder::lossy(50);
    encoder
        .push_backend(AbiAv1StillEncoder::new(late.clone()))
        .push_backend(Scripted::new("next", true, Outcome::Passthrough, &log));
    let out = encode(&encoder).expect("the next backend takes over");
    assert_eq!(out, encode(&AvifEncoder::lossy(50)).unwrap());
    assert!(
        late.0.lock().unwrap().seen_image.is_some(),
        "encode was attempted"
    );
    assert_eq!(
        events(&log),
        vec!["next:supports".to_string(), "next:encode".to_string()]
    );

    // With no later backend, the late decline reaches the built-in tail.
    let mut solo = AvifEncoder::new();
    solo.push_backend(AbiAv1StillEncoder::new(SharedStub(Arc::new(Mutex::new(
        AbiStub::new(Status::OK, Status::UNSUPPORTED, vec![]),
    )))));
    assert_eq!(encode(&solo).unwrap(), encode(&AvifEncoder::new()).unwrap());
}

/// Any other non-OK status is terminal and propagates as a typed error — the tail is not used.
#[test]
fn abi_adapter_other_status_propagates() {
    let log = log();
    let mut encoder = AvifEncoder::new();
    encoder
        .push_backend(AbiAv1StillEncoder::new(SharedStub(Arc::new(Mutex::new(
            AbiStub::new(Status::OK, Status(-42), vec![]),
        )))))
        .push_backend(Scripted::new("never", true, Outcome::Passthrough, &log));
    let err = encode(&encoder).expect_err("a terminal backend status propagates");
    assert_owned_error(
        &err,
        ErrorKind::InvalidInput,
        "AVIF: AV1 encode backend failed",
    );
    assert_eq!(err.detail(), Some("codec-abi status -42"));
    assert!(events(&log).is_empty(), "no later backend, no tail");
}

/// `into_inner` returns the wrapped encoder, and `Debug` works without requiring it of the backend.
#[test]
fn abi_adapter_exposes_the_wrapped_encoder() {
    let stub = SharedStub(Arc::new(Mutex::new(AbiStub::new(
        Status::OK,
        Status::OK,
        vec![],
    ))));
    let adapter = AbiAv1StillEncoder::new(stub.clone());
    assert!(format!("{adapter:?}").contains("AbiAv1StillEncoder"));
    let inner = adapter.into_inner();
    assert!(
        Arc::ptr_eq(&inner.0, &stub.0),
        "the same encoder comes back"
    );
}

#[test]
fn backend_stream_may_use_the_professional_profile() {
    // `seq_profile` 2 is 4:2:2, which the parser must read rather than refuse — the layout of
    // `color_config()` depends on the profile, so accepting it means implementing its branches.
    // Proven by feeding a *valid* profile-2 header to a 4:2:0 request and getting the chroma
    // mismatch, which is a check strictly after the profile gate: a parser that still rejected
    // profile 2 outright would report the profile error instead.
    let mut w = BitVec::default();
    w.push_bits(2, 3); // seq_profile = Professional
    w.push_bits(1, 1); // still_picture
    w.push_bits(1, 1); // reduced_still_picture_header
    w.push_bits(0, 5); // seq_level_idx[0]
    w.push_bits(5, 4); // frame_width_bits_minus_1
    w.push_bits(4, 4); // frame_height_bits_minus_1
    w.push_bits(W - 1, 6);
    w.push_bits(H - 1, 5);
    w.push_bits(0, 6); // the six enable flags
    w.push_bits(0, 1); // high_bitdepth
    w.push_bits(0, 1); // mono_chrome (coded because the profile is not High)
    w.push_bits(1, 1); // color_description_present_flag
    w.push_bits(1, 8); // color_primaries = BT.709
    w.push_bits(13, 8); // transfer_characteristics = sRGB
    w.push_bits(1, 8); // matrix_coefficients = BT.709 (so no sRGB shortcut)
    w.push_bits(1, 1); // color_range = full
    // 4:2:2 codes no chroma_sample_position.
    w.push_bits(0, 1); // separate_uv_delta_q
    let payload = w.finish();
    let mut obus = vec![0x0A, payload.len() as u8];
    obus.extend_from_slice(&payload);

    let log = log();
    let mut encoder = AvifEncoder::lossy(50); // asks for 4:2:0
    encoder.push_backend(Scripted::new("p2", true, Outcome::Bytes(obus), &log));
    let err = encode(&encoder).expect_err("4:2:2 stream for a 4:2:0 request is rejected");
    assert_owned_error(
        &err,
        ErrorKind::InvalidInput,
        "AVIF: AV1 backend stream signals a different chroma format than requested",
    );
}
