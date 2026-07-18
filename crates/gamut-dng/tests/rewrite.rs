//! The preserving rewrite (issue #263): nothing is dropped through open → edit → write —
//! vendor/unknown tags survive byte-exactly, pixel payloads are copied verbatim, the maker-note
//! byte range stays untouched at its original offset, and the Adobe SDK still accepts the
//! result.

mod common;

use gamut_dng::{DngEncoder, DngRewrite, MakerNotePreservation, deconstruct};
use gamut_ifd::{Ifd, UnknownValue, Value, tags as ifd_tags};

/// A small encoded CFA DNG to rewrite.
fn sample_dng() -> Vec<u8> {
    let raw = common::sample_raw(32, 24, 16);
    let mut dng = Vec::new();
    DngEncoder::new()
        .encode(&raw, &common::sample_profile(), &mut dng)
        .expect("encode");
    dng
}

#[test]
fn unedited_rewrite_preserves_the_typed_view_and_classifies_fully() {
    let original = sample_dng();
    let rewrite = DngRewrite::open(&original).expect("open");
    let out = rewrite.write().expect("write");
    assert_eq!(out.maker_note, MakerNotePreservation::Absent);

    // The typed views agree: same raw image, profile, version, metadata.
    let before = gamut_dng::DngDecoder::new()
        .decode(&original)
        .expect("decode original");
    let after = gamut_dng::DngDecoder::new()
        .decode(&out.bytes)
        .expect("decode rewrite");
    assert_eq!(after.raw, before.raw);
    assert_eq!(after.dng_version, before.dng_version);

    // Every byte of the rewritten stream classifies, and the Adobe SDK accepts it.
    let report = deconstruct(&out.bytes).expect("deconstruct");
    assert!(report.is_fully_accounted(), "{report:?}");
    gamut_dng_oracle::validate_dng(&out.bytes).expect("Adobe SDK accepts the rewrite");

    // The rewrite is a fixpoint: rewriting the rewrite is byte-identical.
    let again = DngRewrite::open(&out.bytes)
        .expect("reopen")
        .write()
        .expect("rewrite");
    assert_eq!(again.bytes, out.bytes, "canonical rewrite fixpoint");
}

#[test]
fn vendor_and_unknown_material_survives_a_rewrite() {
    let original = sample_dng();
    let mut rewrite = DngRewrite::open(&original).expect("open");

    // Inject vendor material: a private tag with a known type, and a verbatim unknown-type
    // record — the classes the old decode → encode path dropped.
    let vendor_blob: Vec<u8> = (0..300u16).map(|i| (i % 251) as u8).collect();
    let file = rewrite.file_mut();
    file.ifds[0].set(0x9999, Value::Undefined(vendor_blob.clone()));
    let unknown = UnknownValue::new(0xF0, 2, &[0xAA, 0xBB, 0xCC, 0xDD], file.order, file.variant)
        .expect("capture");
    file.ifds[0].set(0x999A, Value::Unknown(unknown));

    let out = rewrite.write().expect("write");
    let reread = DngRewrite::open(&out.bytes).expect("reopen");
    assert_eq!(
        reread.file().ifds[0].get(0x9999).and_then(Value::as_bytes),
        Some(&vendor_blob[..]),
        "vendor tag value byte-exact"
    );
    assert_eq!(
        reread.file().ifds[0].get(0x999A),
        Some(&Value::Unknown(unknown)),
        "unknown-type record verbatim"
    );
    // The raw image still decodes identically (payloads untouched by the tag edits).
    assert_eq!(
        gamut_dng::DngDecoder::new()
            .decode(&out.bytes)
            .expect("decode")
            .raw,
        gamut_dng::DngDecoder::new()
            .decode(&original)
            .expect("decode original")
            .raw,
    );
}

/// An unsatisfiable pin (the directory region outgrew the note's old offset) falls back to
/// relocation — reported as such, with the note's bytes still exact.
#[test]
fn unsatisfiable_pin_relocates_with_bytes_intact() {
    let note: Vec<u8> = (100..164u8).collect();
    let base = {
        let mut r = DngRewrite::open(&sample_dng()).expect("open");
        let mut exif = Ifd::new();
        exif.set(ifd_tags::MAKER_NOTE, Value::Undefined(note.clone()));
        r.file_mut().ifds[0].set_sub_ifd(ifd_tags::EXIF_IFD, vec![exif]);
        r.write().expect("write").bytes
    };

    let mut r = DngRewrite::open(&base).expect("reopen");
    // Balloon IFD 0 far past the note's old position so the pin cannot be honored.
    for tag in 0x8100..0x8400u16 {
        r.file_mut().ifds[0].set(tag, Value::Long(vec![u32::from(tag)]));
    }
    let out = r.write().expect("write");
    assert_eq!(out.maker_note, MakerNotePreservation::Relocated);
    let reread = DngRewrite::open(&out.bytes).expect("reopen");
    let exif_group = reread.file().ifds[0]
        .sub_ifds()
        .iter()
        .find(|g| g.tag == ifd_tags::EXIF_IFD)
        .expect("exif sub-IFD");
    assert_eq!(
        exif_group.ifds[0]
            .get(ifd_tags::MAKER_NOTE)
            .and_then(Value::as_bytes),
        Some(&note[..]),
        "bytes exact despite relocation"
    );
}

#[test]
fn maker_note_pins_at_its_original_offset_across_an_edit() {
    // Build a DNG carrying a maker note: rewrite the sample, adding an Exif sub-IFD with a
    // vendor blob whose content mimics absolute-offset-sensitive data.
    let note: Vec<u8> = (0..64u8).collect();
    let base = {
        let mut r = DngRewrite::open(&sample_dng()).expect("open");
        let file = r.file_mut();
        let mut exif = Ifd::new();
        exif.set(ifd_tags::MAKER_NOTE, Value::Undefined(note.clone()));
        exif.set(33434, Value::Rational(vec![(1, 100)])); // ExposureTime
        file.ifds[0].set_sub_ifd(ifd_tags::EXIF_IFD, vec![exif]);
        r.write().expect("write").bytes
    };

    // Where did the note land?
    let note_at = base
        .windows(note.len())
        .position(|w| w == &note[..])
        .expect("note embedded");

    // Rewrite with an edit that shifts the layout (a long new tag before the note's directory):
    // the note must stay at exactly its original absolute offset.
    let mut r = DngRewrite::open(&base).expect("reopen");
    r.file_mut().ifds[0].set(
        270, // ImageDescription
        Value::Ascii("an edit long enough to shift every directory after it".to_owned()),
    );
    let out = r.write().expect("write");
    assert_eq!(out.maker_note, MakerNotePreservation::Pinned);
    assert_eq!(
        &out.bytes[note_at..note_at + note.len()],
        &note[..],
        "maker-note byte range untouched at its source offset"
    );

    // And the result still classifies fully (the pin's filler is declared padding).
    let report = deconstruct(&out.bytes).expect("deconstruct");
    assert!(report.segments.is_fully_classified(), "{report:?}");
}
