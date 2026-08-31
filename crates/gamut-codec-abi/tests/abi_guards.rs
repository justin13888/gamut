//! integration · example — the two forward-compatibility guards: `struct_size` leading every
//! descriptor, and `abi_version` leading every vtable.
//!
//! Both exist so a descriptor or vtable built against a different revision of this crate is
//! *detected* rather than reinterpreted field-by-field. Each guard is pinned in both directions:
//! the value a current build produces, and the rejection of a value it did not.

mod common;

use core::ptr;

use common::{TEST_CODEC, one_plane};
use gamut_codec_abi::bridge::{ForeignDecoder, ForeignEncoder};
use gamut_codec_abi::{
    ABI_VERSION, DecoderVTable, EncodeConfig, EncoderVTable, ImageDesc, StreamConfig,
};

#[test]
fn stream_config_struct_size_matches_the_compiled_layout() {
    let cfg = StreamConfig::new(TEST_CODEC, 640, 480, 10);

    assert_eq!(cfg.struct_size, size_of::<StreamConfig>());
    assert!(cfg.is_abi_current());
}

#[test]
fn stream_config_rejects_a_struct_size_from_either_side_of_current() {
    let cfg = StreamConfig::new(TEST_CODEC, 640, 480, 10);

    // A smaller size is an older peer; a larger one is a newer peer. Neither may be read as
    // current, so the guard is an equality test rather than a lower bound.
    let mut stale = cfg;
    stale.struct_size = size_of::<StreamConfig>() - 8;
    assert!(!stale.is_abi_current());

    let mut newer = cfg;
    newer.struct_size = size_of::<StreamConfig>() + 8;
    assert!(!newer.is_abi_current());
}

#[test]
fn encode_config_struct_size_matches_the_compiled_layout() {
    let cfg = EncodeConfig::new(TEST_CODEC, 80);

    assert_eq!(cfg.struct_size, size_of::<EncodeConfig>());
    assert!(cfg.is_abi_current());
}

#[test]
fn encode_config_rejects_a_struct_size_from_either_side_of_current() {
    let cfg = EncodeConfig::new(TEST_CODEC, 80);

    let mut stale = cfg;
    stale.struct_size = 4;
    assert!(!stale.is_abi_current());

    let mut newer = cfg;
    newer.struct_size = size_of::<EncodeConfig>() + 8;
    assert!(!newer.is_abi_current());
}

#[test]
fn image_desc_struct_size_matches_the_compiled_layout() {
    let mut buf = [0u8; 8];
    let desc = one_plane(4, 2, buf.as_mut_ptr(), 4);

    assert_eq!(desc.struct_size, size_of::<ImageDesc>());
    assert!(desc.is_abi_current());
}

#[test]
fn image_desc_rejects_a_struct_size_from_either_side_of_current() {
    let mut buf = [0u8; 8];
    let desc = one_plane(4, 2, buf.as_mut_ptr(), 4);

    let mut stale = desc;
    stale.struct_size = 16;
    assert!(!stale.is_abi_current());

    let mut newer = desc;
    newer.struct_size = size_of::<ImageDesc>() + 8;
    assert!(!newer.is_abi_current());
}

#[test]
fn foreign_decoder_rejects_a_vtable_whose_abi_version_is_not_current() {
    let bumped = DecoderVTable {
        abi_version: ABI_VERSION + 1,
        supports: None,
        decode: None,
        destroy: None,
    };
    // SAFETY: `bumped` is a valid vtable; the null ctx is never used because construction fails.
    assert!(unsafe { ForeignDecoder::new(&bumped, ptr::null_mut()) }.is_none());

    // Zero is what an uninitialised or zeroed C struct carries, so it must not pass either.
    let zero = DecoderVTable {
        abi_version: 0,
        supports: None,
        decode: None,
        destroy: None,
    };
    // SAFETY: as above.
    assert!(unsafe { ForeignDecoder::new(&zero, ptr::null_mut()) }.is_none());
}

#[test]
fn foreign_encoder_rejects_a_vtable_whose_abi_version_is_not_current() {
    let bumped = EncoderVTable {
        abi_version: ABI_VERSION + 1,
        supports: None,
        encode: None,
        destroy: None,
    };
    // SAFETY: `bumped` is a valid vtable; the null ctx is never used because construction fails.
    assert!(unsafe { ForeignEncoder::new(&bumped, ptr::null_mut()) }.is_none());

    let zero = EncoderVTable {
        abi_version: 0,
        supports: None,
        encode: None,
        destroy: None,
    };
    // SAFETY: as above.
    assert!(unsafe { ForeignEncoder::new(&zero, ptr::null_mut()) }.is_none());
}

#[test]
fn the_foreign_constructors_reject_a_null_vtable() {
    // A null vtable is explicitly allowed as *input* by the constructors' contract — it is the
    // shape a C caller passes when it has no backend — and must read as "no adapter", not a
    // dereference.
    // SAFETY: passing null is exactly what the constructor's contract permits.
    assert!(unsafe { ForeignDecoder::new(ptr::null(), ptr::null_mut()) }.is_none());
    // SAFETY: as above.
    assert!(unsafe { ForeignEncoder::new(ptr::null(), ptr::null_mut()) }.is_none());
}
