//! Differential conformance against exiv2's EXIF parser/serializer, the engine the crate documents
//! as its oracle. These cross-checks complement the spec-derived golden vectors: the golden tests
//! pin exact bytes, while these prove gamut interoperates with the reference implementation — its
//! output is real EXIF that exiv2 reads, and it reads exiv2's output back. Equality is asserted on
//! re-parsed tag *values*, not on exiv2's bytes (exiv2 lays out and orders the stream its own way).
//!
//! Requires the `third_party/exiv2` + `third_party/expat` submodules and a C++ toolchain.

use gamut_exif::{ByteOrder, Exif, ExifTag, ExifWriter, Rational, Value};

/// A representative model spanning the 0th IFD and the Exif sub-IFD.
fn sample() -> Exif {
    let mut exif = Exif::new(ByteOrder::LittleEndian);
    exif.set_tag(ExifTag::Make, Value::Ascii("Canon".into()));
    exif.set_tag(ExifTag::Model, Value::Ascii("Canon EOS R5".into()));
    exif.set_tag(ExifTag::Orientation, Value::Short(vec![1]));
    exif.set_tag(ExifTag::ExifVersion, Value::Undefined(b"0300".to_vec()));
    exif.set_tag(ExifTag::FNumber, Value::Rational(vec![(28, 10)]));
    exif.set_tag(ExifTag::ExposureTime, Value::Rational(vec![(1, 250)]));
    exif.set_tag(ExifTag::PhotographicSensitivity, Value::Short(vec![400]));
    exif.set_tag(
        ExifTag::DateTimeOriginal,
        Value::Ascii("2024:01:01 12:00:00".into()),
    );
    exif
}

/// The bare TIFF stream (no `Exif\0\0` marker) that exiv2's `ExifParser` consumes.
fn bare(exif: &Exif) -> Vec<u8> {
    ExifWriter::new().marker(false).write(exif)
}

#[test]
fn exiv2_reads_the_standard_tags_gamut_wrote() {
    let bytes = bare(&sample());

    // exiv2 parses the whole stream — the 0th IFD and the Exif sub-IFD behind the ExifIFD pointer.
    let count = exiv2_oracle::exif_count(&bytes).expect("exiv2 decodes gamut's EXIF");
    assert!(count >= 8, "exiv2 read only {count} tags");

    // Values round-trip exactly through the reference reader.
    let get = |key: &str| exiv2_oracle::exif_get(&bytes, key).expect(key);
    assert_eq!(get("Exif.Image.Make"), "Canon");
    assert_eq!(get("Exif.Image.Model"), "Canon EOS R5");
    assert_eq!(get("Exif.Image.Orientation"), "1");
    assert_eq!(get("Exif.Photo.FNumber"), "28/10");
    assert_eq!(get("Exif.Photo.ExposureTime"), "1/250");
    assert_eq!(get("Exif.Photo.ISOSpeedRatings"), "400");
    assert_eq!(get("Exif.Photo.DateTimeOriginal"), "2024:01:01 12:00:00");
}

#[test]
fn gamut_reads_what_exiv2_writes() {
    let original = sample();
    // exiv2 re-encodes the stream into its own canonical layout...
    let exiv2_bytes = exiv2_oracle::exif_roundtrip(&bare(&original)).expect("exiv2 re-encodes");
    // ...and gamut must read the same values back out of it.
    let parsed = Exif::parse(&exiv2_bytes).expect("gamut parses exiv2's EXIF");

    assert_eq!(parsed.make(), Some("Canon"));
    assert_eq!(parsed.model(), Some("Canon EOS R5"));
    assert_eq!(parsed.orientation(), Some(1));
    assert_eq!(parsed.f_number(), Some(Rational { num: 28, den: 10 }));
    assert_eq!(parsed.exposure_time(), Some(Rational { num: 1, den: 250 }));
    assert_eq!(parsed.iso(), Some(400));
    assert_eq!(parsed.datetime_original(), Some("2024:01:01 12:00:00"));
}
