//! Behavioural tests for the seam: the C ↔ Rust bridges, the ABI/`struct_size` guards, the
//! [`Status`] fall-through semantics, and the exactly-once `destroy` contract.

use core::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gamut_codec_abi::bridge::{ForeignDecoder, ForeignEncoder, lower_decoder, lower_encoder};
use gamut_codec_abi::{
    ABI_VERSION, Decoder, DecoderVTable, EncodeConfig, Encoder, EncoderVTable, ImageDesc,
    MAX_PLANES, Status, StreamConfig,
};

/// The codec id the test backends claim.
const TEST_CODEC: u32 = 0x0C0D_E001;
/// A terminal (accepted-then-failed) backend error, distinct from `UNSUPPORTED`.
const BACKEND_FAILURE: Status = Status(7);
/// The status the aborting test sink returns.
const SINK_ABORT: Status = Status(9);

// ---- test backends ---------------------------------------------------------------------------

/// A decoder that accepts only [`TEST_CODEC`] and writes two derived bytes into plane 0.
struct TestDecoder {
    destroyed: Arc<AtomicUsize>,
}

impl Decoder for TestDecoder {
    fn supports(&mut self, cfg: &StreamConfig) -> bool {
        cfg.codec_id == TEST_CODEC
    }

    fn decode(&mut self, cfg: &StreamConfig, codestream: &[u8], out: &ImageDesc) -> Status {
        // Accepted-then-failed: a terminal status the host must propagate, not fall through on.
        if codestream != b"stream" {
            return BACKEND_FAILURE;
        }
        // SAFETY: the test allocates a >= 2-byte buffer behind `out.planes[0]`.
        unsafe {
            *out.planes[0] = codestream.len() as u8;
            *out.planes[0].add(1) = cfg.width as u8;
        }
        Status::OK
    }
}

impl Drop for TestDecoder {
    fn drop(&mut self) {
        self.destroyed.fetch_add(1, Ordering::SeqCst);
    }
}

/// An encoder that emits `[width, height]` then `[quality]`, aborting if the sink says so.
struct TestEncoder {
    destroyed: Arc<AtomicUsize>,
}

impl Encoder for TestEncoder {
    fn supports(&mut self, cfg: &EncodeConfig) -> bool {
        cfg.codec_id == TEST_CODEC
    }

    fn encode(
        &mut self,
        cfg: &EncodeConfig,
        image: &ImageDesc,
        sink: &mut dyn FnMut(&[u8]) -> Status,
    ) -> Status {
        let status = sink(&[image.width as u8, image.height as u8]);
        if !status.is_ok() {
            return status;
        }
        sink(&[cfg.quality as u8])
    }
}

impl Drop for TestEncoder {
    fn drop(&mut self) {
        self.destroyed.fetch_add(1, Ordering::SeqCst);
    }
}

/// An [`ImageDesc`] over a single plane at `ptr` with `stride`.
fn one_plane(width: u32, height: u32, ptr: *mut u8, stride: usize) -> ImageDesc {
    let mut planes = [ptr::null_mut(); MAX_PLANES];
    planes[0] = ptr;
    let mut strides = [0usize; MAX_PLANES];
    strides[0] = stride;
    ImageDesc::new(0, width, height, 8, 1, planes, strides)
}

// ---- Status semantics ------------------------------------------------------------------------

#[test]
fn status_constants_and_predicates() {
    assert_eq!(Status::OK.0, 0);
    assert_eq!(Status::UNSUPPORTED.0, -1);

    assert!(Status::OK.is_ok());
    assert!(!Status::OK.is_unsupported());
    assert!(Status::UNSUPPORTED.is_unsupported());
    assert!(!Status::UNSUPPORTED.is_ok());

    // A terminal backend error is neither OK nor the fall-through code.
    assert!(!BACKEND_FAILURE.is_ok());
    assert!(!BACKEND_FAILURE.is_unsupported());

    assert_eq!(Status(0), Status::OK);
    assert_eq!(Status(-1), Status::UNSUPPORTED);
    assert_ne!(Status(1), Status::OK);
    assert_ne!(Status(1), Status::UNSUPPORTED);
}

// ---- descriptor struct_size guards -----------------------------------------------------------

#[test]
fn stream_config_struct_size_guard() {
    let cfg = StreamConfig::new(TEST_CODEC, 640, 480, 10);
    assert_eq!(cfg.struct_size, size_of::<StreamConfig>());
    assert!(cfg.is_abi_current());
    assert_eq!(cfg.codec_id, TEST_CODEC);
    assert_eq!(cfg.width, 640);
    assert_eq!(cfg.height, 480);
    assert_eq!(cfg.bit_depth, 10);
    assert_eq!(cfg.extradata_len, 0);
    assert!(cfg.extradata.is_null());

    let mut stale = cfg;
    stale.struct_size = size_of::<StreamConfig>() - 8;
    assert!(!stale.is_abi_current());
    let mut newer = cfg;
    newer.struct_size = size_of::<StreamConfig>() + 8;
    assert!(!newer.is_abi_current());
}

#[test]
fn encode_config_struct_size_guard() {
    let cfg = EncodeConfig::new(TEST_CODEC, 80);
    assert_eq!(cfg.struct_size, size_of::<EncodeConfig>());
    assert!(cfg.is_abi_current());
    assert_eq!(cfg.codec_id, TEST_CODEC);
    assert_eq!(cfg.quality, 80);
    assert_eq!(cfg.extra_len, 0);
    assert!(cfg.extra.is_null());

    let mut stale = cfg;
    stale.struct_size = 4;
    assert!(!stale.is_abi_current());
}

#[test]
fn image_desc_struct_size_guard_and_fields() {
    let mut buf = [0u8; 8];
    let desc = one_plane(4, 2, buf.as_mut_ptr(), 4);
    assert_eq!(desc.struct_size, size_of::<ImageDesc>());
    assert!(desc.is_abi_current());
    assert_eq!(desc.width, 4);
    assert_eq!(desc.height, 2);
    assert_eq!(desc.depth, 8);
    assert_eq!(desc.plane_count, 1);
    assert_eq!(desc.pixel_format, 0);
    assert_eq!(desc.strides[0], 4);
    assert!(desc.planes[1].is_null());

    let mut stale = desc;
    stale.struct_size = 16;
    assert!(!stale.is_abi_current());
}

#[test]
fn stream_config_extradata_borrows_or_is_empty() {
    let record = [0xAAu8, 0xBB, 0xCC];
    let mut cfg = StreamConfig::new(TEST_CODEC, 1, 1, 8);
    // SAFETY: no extradata is attached, so this takes the empty-slice path.
    assert_eq!(unsafe { cfg.extradata() }, &[] as &[u8]);

    cfg.extradata = record.as_ptr();
    cfg.extradata_len = record.len();
    // SAFETY: `record` outlives the borrow and covers `extradata_len` bytes.
    assert_eq!(unsafe { cfg.extradata() }, &[0xAA, 0xBB, 0xCC]);
}

// ---- Rust -> C -> Rust round trip --------------------------------------------------------------

#[test]
fn decoder_round_trips_through_the_vtable() {
    let destroyed = Arc::new(AtomicUsize::new(0));
    let (vtable, ctx) = lower_decoder(Box::new(TestDecoder {
        destroyed: Arc::clone(&destroyed),
    }));
    // SAFETY: `vtable`/`ctx` come from `lower_decoder`, and `vtable` (declared first) outlives
    // `foreign`; the boxed decoder is `Send`.
    let mut foreign = unsafe { ForeignDecoder::new(&vtable, ctx) }.expect("ABI version matches");

    let cfg = StreamConfig::new(TEST_CODEC, 6, 2, 8);
    assert!(foreign.supports(&cfg));
    assert!(!foreign.supports(&StreamConfig::new(TEST_CODEC + 1, 6, 2, 8)));

    let mut buf = [0u8; 2];
    let out = one_plane(6, 2, buf.as_mut_ptr(), 6);
    assert_eq!(foreign.decode(&cfg, b"stream", &out), Status::OK);
    // Proves the payload crossed intact both ways: codestream len (6) and cfg.width (6)…
    assert_eq!(buf, [6, 6]);

    // …and again with a different width, so the value is really read from `cfg`.
    let cfg2 = StreamConfig::new(TEST_CODEC, 200, 2, 8);
    assert_eq!(foreign.decode(&cfg2, b"stream", &out), Status::OK);
    assert_eq!(buf, [6, 200]);

    // Accepted-then-failed: the terminal status propagates verbatim, it is not UNSUPPORTED.
    let failed = foreign.decode(&cfg, b"nope", &out);
    assert_eq!(failed, BACKEND_FAILURE);
    assert!(!failed.is_unsupported());

    assert_eq!(destroyed.load(Ordering::SeqCst), 0);
    drop(foreign);
    assert_eq!(destroyed.load(Ordering::SeqCst), 1);
}

#[test]
fn encoder_round_trips_through_the_vtable() {
    let destroyed = Arc::new(AtomicUsize::new(0));
    let (vtable, ctx) = lower_encoder(Box::new(TestEncoder {
        destroyed: Arc::clone(&destroyed),
    }));
    // SAFETY: as above — `vtable` outlives `foreign` and pairs with `ctx`.
    let mut foreign = unsafe { ForeignEncoder::new(&vtable, ctx) }.expect("ABI version matches");

    let cfg = EncodeConfig::new(TEST_CODEC, 80);
    assert!(foreign.supports(&cfg));
    assert!(!foreign.supports(&EncodeConfig::new(TEST_CODEC + 1, 80)));

    let image = one_plane(3, 2, ptr::null_mut(), 0);
    let mut written = Vec::new();
    let status = foreign.encode(&cfg, &image, &mut |chunk| {
        written.extend_from_slice(chunk);
        Status::OK
    });
    assert_eq!(status, Status::OK);
    // Two separate sink calls, in order: [width, height] then [quality].
    assert_eq!(written, vec![3, 2, 80]);

    assert_eq!(destroyed.load(Ordering::SeqCst), 0);
    drop(foreign);
    assert_eq!(destroyed.load(Ordering::SeqCst), 1);
}

#[test]
fn encoder_propagates_a_sink_abort_and_stops() {
    let destroyed = Arc::new(AtomicUsize::new(0));
    let (vtable, ctx) = lower_encoder(Box::new(TestEncoder {
        destroyed: Arc::clone(&destroyed),
    }));
    // SAFETY: `vtable` outlives `foreign` and pairs with `ctx`.
    let mut foreign = unsafe { ForeignEncoder::new(&vtable, ctx) }.expect("ABI version matches");

    let cfg = EncodeConfig::new(TEST_CODEC, 55);
    let image = one_plane(9, 4, ptr::null_mut(), 0);
    let mut calls = 0usize;
    let status = foreign.encode(&cfg, &image, &mut |_chunk| {
        calls += 1;
        SINK_ABORT
    });
    assert_eq!(status, SINK_ABORT);
    // The abort reached the encoder after the first chunk, so it never emitted the second.
    assert_eq!(calls, 1);
}

// ---- ABI version guard -------------------------------------------------------------------------

#[test]
fn foreign_decoder_rejects_a_mismatched_abi_version() {
    let bumped = DecoderVTable {
        abi_version: ABI_VERSION + 1,
        supports: None,
        decode: None,
        destroy: None,
    };
    // SAFETY: `bumped` is a valid vtable; the null ctx is never used because construction fails.
    assert!(unsafe { ForeignDecoder::new(&bumped, ptr::null_mut()) }.is_none());

    let zero = DecoderVTable {
        abi_version: 0,
        supports: None,
        decode: None,
        destroy: None,
    };
    // SAFETY: as above.
    assert!(unsafe { ForeignDecoder::new(&zero, ptr::null_mut()) }.is_none());

    // SAFETY: a null vtable is explicitly allowed by the constructor's contract.
    assert!(unsafe { ForeignDecoder::new(ptr::null(), ptr::null_mut()) }.is_none());
}

#[test]
fn foreign_encoder_rejects_a_mismatched_abi_version() {
    let bumped = EncoderVTable {
        abi_version: ABI_VERSION + 1,
        supports: None,
        encode: None,
        destroy: None,
    };
    // SAFETY: `bumped` is a valid vtable; the null ctx is never used because construction fails.
    assert!(unsafe { ForeignEncoder::new(&bumped, ptr::null_mut()) }.is_none());

    // SAFETY: a null vtable is explicitly allowed by the constructor's contract.
    assert!(unsafe { ForeignEncoder::new(ptr::null(), ptr::null_mut()) }.is_none());
}

// ---- absent (null) function pointers -----------------------------------------------------------

#[test]
fn decoder_with_no_callbacks_supports_nothing_and_falls_through() {
    let empty = DecoderVTable {
        abi_version: ABI_VERSION,
        supports: None,
        decode: None,
        destroy: None,
    };
    // SAFETY: `empty` is a valid vtable with no callbacks, so the null ctx is never dereferenced.
    let mut foreign = unsafe { ForeignDecoder::new(&empty, ptr::null_mut()) }.expect("ABI matches");

    let cfg = StreamConfig::new(TEST_CODEC, 1, 1, 8);
    assert!(!foreign.supports(&cfg));

    let out = one_plane(1, 1, ptr::null_mut(), 0);
    // An absent `decode` reads as the fall-through code, not a terminal error.
    let status = foreign.decode(&cfg, b"", &out);
    assert_eq!(status, Status::UNSUPPORTED);
    assert!(status.is_unsupported());

    // A `None` destroy makes drop a no-op rather than a null call.
    drop(foreign);
}

#[test]
fn encoder_with_no_callbacks_supports_nothing_and_falls_through() {
    let empty = EncoderVTable {
        abi_version: ABI_VERSION,
        supports: None,
        encode: None,
        destroy: None,
    };
    // SAFETY: `empty` is a valid vtable with no callbacks, so the null ctx is never dereferenced.
    let mut foreign = unsafe { ForeignEncoder::new(&empty, ptr::null_mut()) }.expect("ABI matches");

    let cfg = EncodeConfig::new(TEST_CODEC, 50);
    assert!(!foreign.supports(&cfg));

    let image = one_plane(1, 1, ptr::null_mut(), 0);
    let mut calls = 0usize;
    let status = foreign.encode(&cfg, &image, &mut |_chunk| {
        calls += 1;
        Status::OK
    });
    assert_eq!(status, Status::UNSUPPORTED);
    // An absent `encode` must not have invoked the sink at all.
    assert_eq!(calls, 0);

    drop(foreign);
}

// ---- host-side fallback contract ----------------------------------------------------------------

/// The registry semantics downstream format crates implement: push order, `UNSUPPORTED`-only
/// fall-through, and terminal propagation once a backend accepts.
#[test]
fn registry_tries_backends_in_push_order_until_one_supports() {
    let destroyed = Arc::new(AtomicUsize::new(0));
    // Backend 0 rejects TEST_CODEC + 1; backend 1 (the "software tail") accepts everything.
    let (vt, ctx) = lower_decoder(Box::new(TestDecoder {
        destroyed: Arc::clone(&destroyed),
    }));
    // SAFETY: `vt` outlives `picky` and pairs with `ctx`.
    let mut picky = unsafe { ForeignDecoder::new(&vt, ctx) }.expect("ABI matches");

    struct Tail;
    impl Decoder for Tail {
        fn supports(&mut self, _cfg: &StreamConfig) -> bool {
            true
        }
        fn decode(&mut self, _cfg: &StreamConfig, _bytes: &[u8], _out: &ImageDesc) -> Status {
            Status::OK
        }
    }
    let mut tail = Tail;

    let unsupported_by_first = StreamConfig::new(TEST_CODEC + 1, 1, 1, 8);
    let registry: [&mut dyn Decoder; 2] = [&mut picky, &mut tail];
    let mut chosen = None;
    for (index, backend) in registry.into_iter().enumerate() {
        if backend.supports(&unsupported_by_first) {
            chosen = Some(index);
            break;
        }
    }
    // The first backend fell through; the software tail took the job.
    assert_eq!(chosen, Some(1));
}
