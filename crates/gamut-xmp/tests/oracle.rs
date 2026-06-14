//! Differential conformance against exiv2's bundled Adobe XMP Toolkit (XMPCore), the engine the
//! crate documents as its oracle. These cross-checks complement the spec-derived golden vectors:
//! the golden tests pin the exact canonical bytes, while these prove gamut interoperates with the
//! reference implementation — its output is real XMP the toolkit reads, and it reads the toolkit's
//! output back. Equality is asserted on the re-parsed graph's values, not on exiv2's bytes (XMPCore
//! normalizes namespace and field ordering its own way).
//!
//! Requires the `third_party/exiv2` + `third_party/expat` submodules and a C++ toolchain.

use gamut_xmp::{XmpArray, XmpItem, XmpMeta, XmpProperty, XmpValue};

const DC: &str = "http://purl.org/dc/elements/1.1/";
const XMP: &str = "http://ns.adobe.com/xap/1.0/";

/// A graph covering simple, URI, Bag, and language-alternative values.
fn sample() -> XmpMeta {
    let mut meta = XmpMeta::new();
    meta.set_text(DC, "format", "text/plain");
    meta.set(XmpProperty::new(
        XMP,
        "BaseURL",
        XmpValue::Uri("http://example.com/".into()),
    ));
    meta.set(XmpProperty::new(
        DC,
        "subject",
        XmpValue::Array(XmpArray::Bag(vec![
            XmpItem::new(XmpValue::Simple("alpha".into())),
            XmpItem::new(XmpValue::Simple("beta".into())),
        ])),
    ));
    meta.set_lang_alt(DC, "title", "x-default", "Hello");
    meta
}

#[test]
fn gamut_output_is_valid_and_readable_by_exiv2() {
    let packet = sample().to_packet();
    exiv2_oracle::validate(&packet).expect("exiv2 (Adobe XMPCore) must accept gamut's packet");

    // Specific values survive into the reference engine.
    assert_eq!(
        exiv2_oracle::get_property(&packet, "Xmp.dc.format").unwrap(),
        "text/plain"
    );
    assert_eq!(
        exiv2_oracle::get_property(&packet, "Xmp.xmp.BaseURL").unwrap(),
        "http://example.com/"
    );
    // The bag items survive as a single keyed entry plus its members.
    assert!(exiv2_oracle::property_count(&packet).unwrap() >= 3);
}

#[test]
fn gamut_reads_exiv2_reserialization() {
    // exiv2 re-serializes via XMPCore; gamut must parse that back to the same values.
    let canonical = exiv2_oracle::roundtrip(&sample().to_packet()).expect("exiv2 round-trip");
    let parsed = XmpMeta::from_packet(&canonical).expect("gamut parses exiv2's output");

    assert_eq!(parsed.get_text(DC, "format"), Some("text/plain"));
    // The URL's value survives the round-trip; exiv2 normalizes the URI form to element text, so we
    // assert on the data rather than the `Uri` vs `Simple` form.
    assert_eq!(parsed.get_text(XMP, "BaseURL"), Some("http://example.com/"));
    assert_eq!(parsed.get_lang_alt(DC, "title", "x-default"), Some("Hello"));

    let XmpValue::Array(XmpArray::Bag(items)) = &parsed.get(DC, "subject").unwrap().value else {
        panic!("dc:subject must round-trip through exiv2 as a Bag");
    };
    let values: Vec<&str> = items.iter().filter_map(XmpItem::text).collect();
    assert_eq!(values, ["alpha", "beta"]);
}

#[test]
fn empty_packet_round_trips_through_exiv2() {
    // A property-less packet is still valid XMP the toolkit accepts.
    let packet = XmpMeta::new().to_packet();
    exiv2_oracle::validate(&packet).expect("exiv2 must accept an empty packet");
}
