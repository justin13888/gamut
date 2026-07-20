//! Pins the composition the C seam (issue #280 → #242) depends on: one long-lived
//! [`ForeignDecoder`] — pushed once, `destroy` deferred to its drop — lent to a fresh
//! per-item [`AbiHevcDecoder`] for each picture size, via `gamut-codec-abi`'s
//! `&mut D: Decoder` blanket impl.

use std::sync::atomic::{AtomicUsize, Ordering};

use gamut_codec_abi::bridge::ForeignDecoder;
use gamut_codec_abi::{ABI_VERSION, DecoderVTable, ImageDesc, Status, StreamConfig};
use gamut_core::Dimensions;
use gamut_heic::{AbiHevcDecoder, HEVC_CODEC_ID, HevcConfig, HevcDecoder};

/// A minimal `hvcC` record: Main profile, monochrome, 8-bit, 4-byte NAL length prefixes,
/// zero parameter-set arrays.
const HVCC_MONO_8BIT: [u8; 23] = [
    0x01, // configurationVersion
    0x01, // profile_space 0 | tier 0 | profile_idc 1 (Main)
    0x00, 0x00, 0x00, 0x00, // general_profile_compatibility_flags
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00, // general_constraint_indicator_flags
    0x00, // general_level_idc
    0x00, 0x00, // min_spatial_segmentation_idc
    0x00, // parallelismType
    0x00, // chroma_format_idc 0 = monochrome
    0x00, // bit_depth_luma_minus8
    0x00, // bit_depth_chroma_minus8
    0x00, 0x00, // avgFrameRate
    0x0B, // constantFrameRate 0 | numTemporalLayers 1 | temporalIdNested 0 | lengthSizeMinusOne 3
    0x00, // numOfArrays
];

unsafe extern "C" fn supports_shim(
    _ctx: *mut core::ffi::c_void,
    cfg: *const StreamConfig,
) -> Status {
    // SAFETY: the host passes a valid StreamConfig for the duration of the call.
    if unsafe { (*cfg).codec_id } == HEVC_CODEC_ID {
        Status::OK
    } else {
        Status::UNSUPPORTED
    }
}

/// Writes the stream's width into the first luma sample, proving the backend saw the
/// per-adapter dimensions.
unsafe extern "C" fn decode_shim(
    _ctx: *mut core::ffi::c_void,
    cfg: *const StreamConfig,
    _codestream: *const u8,
    _codestream_len: usize,
    out: *const ImageDesc,
) -> Status {
    // SAFETY: the adapter allocated plane 0 as `width * height` u16 samples and both
    // descriptors are valid for the duration of the call.
    unsafe {
        *(*out).planes[0].cast::<u16>() = (*cfg).width as u16;
    }
    Status::OK
}

unsafe extern "C" fn destroy_shim(ctx: *mut core::ffi::c_void) {
    // SAFETY: `ctx` is the test's AtomicUsize destroy counter, which outlives the backend.
    unsafe { &*ctx.cast_const().cast::<AtomicUsize>() }.fetch_add(1, Ordering::SeqCst);
}

const VTABLE: DecoderVTable = DecoderVTable {
    abi_version: ABI_VERSION,
    supports: Some(supports_shim),
    decode: Some(decode_shim),
    destroy: Some(destroy_shim),
};

#[test]
fn borrowed_foreign_decoder_adapts_per_item_and_destroys_once() {
    let destroyed = AtomicUsize::new(0);
    let ctx = std::ptr::from_ref(&destroyed).cast_mut().cast();

    // SAFETY: VTABLE is valid for the program lifetime and ctx outlives the backend.
    let mut foreign = unsafe { ForeignDecoder::new(&VTABLE, ctx) }.expect("current ABI_VERSION");
    let config = HevcConfig::parse(&HVCC_MONO_8BIT).expect("minimal hvcC parses");

    // Two items with different `ispe` dimensions share the one stored backend: each gets a
    // fresh borrowing adapter, none of which takes ownership.
    for (width, height) in [(4u32, 2u32), (8, 3)] {
        let dims = Dimensions::new(width, height).expect("non-zero");
        let mut adapter = AbiHevcDecoder::new(&mut foreign, dims);
        assert!(adapter.supports(&config));
        let frame = adapter.decode_intra(&config, &[]).expect("stub decodes");
        assert_eq!(frame.width(), width);
        assert_eq!(frame.y().len(), (width * height) as usize);
        assert_eq!(frame.y()[0], width as u16, "backend saw this item's width");
    }

    assert_eq!(
        destroyed.load(Ordering::SeqCst),
        0,
        "no adapter ran destroy"
    );
    drop(foreign);
    assert_eq!(
        destroyed.load(Ordering::SeqCst),
        1,
        "owner destroys exactly once"
    );
}
