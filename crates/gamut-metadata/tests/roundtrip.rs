//! Facade integration tests: block dispatch, IPTC↔XMP reconciliation, and the keystone
//! extract→embed→extract round-trip (true equality — the model stores each datum once).

use gamut_metadata::exif::{ByteOrder, Exif, ExifTag, Value};
use gamut_metadata::icc::{ColorSpace, DeviceClass, IccProfile, ProfileHeader};
use gamut_metadata::iptc::{IimBlock, IimCharset, IimDataSet};
use gamut_metadata::xmp::{WellKnownNs, XmpMeta};
use gamut_metadata::{
    ConflictPolicy, EncodedMetadata, Metadata, MetadataBlock, MetadataEmbedder, MetadataError,
    MetadataExtractor,
};

// --- carrier fixtures (each produced by its own leaf crate, so the bytes are genuine) ------------

fn exif_bytes() -> Vec<u8> {
    let mut exif = Exif::new(ByteOrder::LittleEndian);
    exif.set_tag(ExifTag::Make, Value::Ascii("gamut".to_owned()));
    exif.to_bytes()
}

fn icc_bytes() -> Vec<u8> {
    IccProfile {
        header: ProfileHeader::new(DeviceClass::Display, ColorSpace::Rgb),
        tags: Vec::new(),
    }
    .to_bytes()
    .expect("a header-only profile serializes")
}

/// A non-IPTC XMP packet (an `xmp:CreatorTool` property — outside the IPTC namespaces).
fn xmp_bytes_non_iptc() -> Vec<u8> {
    let mut xmp = XmpMeta::new();
    xmp.set_text(WellKnownNs::Xmp.uri(), "CreatorTool", "gamut");
    xmp.to_packet()
}

/// An XMP packet carrying an IPTC `photoshop:City` property.
fn xmp_bytes_city(city: &str) -> Vec<u8> {
    let mut xmp = XmpMeta::new();
    xmp.set_text(WellKnownNs::Photoshop.uri(), "City", city);
    xmp.to_packet()
}

/// A legacy IIM dataset stream carrying `2:90` City.
fn iim_bytes_city(city: &str) -> Vec<u8> {
    IimBlock {
        datasets: vec![IimDataSet {
            record: 2,
            dataset: 90,
            data: city.as_bytes().to_vec(),
        }],
    }
    .encode()
    .expect("iim encodes")
}

/// Rebuild the blocks an `EncodedMetadata` implies and extract them again.
fn reextract(enc: &EncodedMetadata) -> Metadata {
    let mut blocks = Vec::new();
    if let Some(b) = &enc.exif {
        blocks.push(MetadataBlock::Exif(b));
    }
    if let Some(b) = &enc.xmp {
        blocks.push(MetadataBlock::Xmp(b));
    }
    if let Some(b) = &enc.icc {
        blocks.push(MetadataBlock::Icc(b));
    }
    if let Some(b) = &enc.iptc_iim {
        blocks.push(MetadataBlock::IptcIim(b));
    }
    MetadataExtractor::new().extract(&blocks).unwrap()
}

// --- extraction ----------------------------------------------------------------------------------

#[test]
fn extract_dispatches_each_carrier_to_its_field() {
    let (exif, xmp, icc) = (exif_bytes(), xmp_bytes_non_iptc(), icc_bytes());
    let meta = MetadataExtractor::new()
        .extract(&[
            MetadataBlock::Exif(&exif),
            MetadataBlock::Xmp(&xmp),
            MetadataBlock::Icc(&icc),
        ])
        .unwrap();
    assert_eq!(meta.exif.as_ref().and_then(Exif::make), Some("gamut"));
    assert_eq!(
        meta.xmp
            .as_ref()
            .and_then(|x| x.get_text(WellKnownNs::Xmp.uri(), "CreatorTool")),
        Some("gamut")
    );
    assert!(meta.icc.is_some());
}

#[test]
fn extract_empty_yields_empty_metadata() {
    let meta = MetadataExtractor::new().extract(&[]).unwrap();
    assert!(meta.is_empty());
    assert_eq!(meta, Metadata::default());
}

#[test]
fn extract_drops_an_empty_xmp_graph() {
    // An XMP packet that parses to no properties is reported as absent, so `xmp.is_none()` stays
    // meaningful.
    let empty = XmpMeta::new().to_packet();
    let meta = MetadataExtractor::new()
        .extract(&[MetadataBlock::Xmp(&empty)])
        .unwrap();
    assert!(meta.xmp.is_none());
}

#[test]
fn extract_names_the_failing_carrier() {
    let exif_err = MetadataExtractor::new()
        .extract(&[MetadataBlock::Exif(b"not exif")])
        .unwrap_err();
    assert!(matches!(exif_err, MetadataError::Exif(_)));

    let icc_err = MetadataExtractor::new()
        .extract(&[MetadataBlock::Icc(b"short")])
        .unwrap_err();
    assert!(matches!(icc_err, MetadataError::Icc(_)));

    let iptc_err = MetadataExtractor::new()
        .extract(&[MetadataBlock::IptcIim(&[0x00, 0x01])])
        .unwrap_err();
    assert!(matches!(iptc_err, MetadataError::Iptc(_)));
}

// --- reconciliation (P4 read side) ---------------------------------------------------------------

#[test]
fn reconciliation_folds_iim_into_xmp_per_policy() {
    let xmp = xmp_bytes_city("Tokyo");
    let iim = iim_bytes_city("Kyoto");
    let blocks = [MetadataBlock::Xmp(&xmp), MetadataBlock::IptcIim(&iim)];

    // Default XmpWins keeps the modern carrier's value.
    let xmp_wins = MetadataExtractor::new().extract(&blocks).unwrap();
    assert_eq!(xmp_wins.iptc().unwrap().city(), Some("Tokyo"));

    // IimWins prefers the legacy value — and folds it into the single XMP graph.
    let iim_wins = MetadataExtractor::new()
        .policy(ConflictPolicy::IimWins)
        .extract(&blocks)
        .unwrap();
    assert_eq!(iim_wins.iptc().unwrap().city(), Some("Kyoto"));
    assert_eq!(
        iim_wins
            .xmp
            .as_ref()
            .and_then(|x| x.get_text(WellKnownNs::Photoshop.uri(), "City")),
        Some("Kyoto"),
    );
}

#[test]
fn conflicts_reports_carrier_disagreements() {
    let xmp = xmp_bytes_city("Tokyo");
    let iim = iim_bytes_city("Kyoto");

    let conflicts = MetadataExtractor::new()
        .conflicts(&[MetadataBlock::Xmp(&xmp), MetadataBlock::IptcIim(&iim)])
        .unwrap();
    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].field, "City");

    // A single carrier cannot disagree with itself.
    assert!(
        MetadataExtractor::new()
            .conflicts(&[MetadataBlock::Xmp(&xmp)])
            .unwrap()
            .is_empty()
    );
}

// --- embedding (P3) ------------------------------------------------------------------------------

#[test]
fn embed_serializes_only_present_carriers() {
    let meta = Metadata {
        exif: Some(Exif::new(ByteOrder::LittleEndian)),
        xmp: None,
        icc: None,
    };
    let enc = meta.encode().unwrap();
    assert!(enc.exif.is_some());
    assert_eq!(enc.xmp, None);
    assert_eq!(enc.icc, None);
    assert_eq!(enc.iptc_iim, None);

    // A wholly empty model produces no blocks at all.
    assert_eq!(Metadata::default().encode().unwrap(), EncodedMetadata::default());
}

#[test]
fn from_blocks_matches_the_default_extractor() {
    let xmp = xmp_bytes_non_iptc();
    let meta = Metadata::from_blocks(&[MetadataBlock::Xmp(&xmp)]).unwrap();
    assert_eq!(
        meta,
        MetadataExtractor::new()
            .extract(&[MetadataBlock::Xmp(&xmp)])
            .unwrap()
    );
}

#[test]
fn embed_iim_request_without_iptc_produces_no_block() {
    // emit_iptc_iim is set, but the model carries no IPTC data → still nothing to emit.
    let exif = exif_bytes();
    let meta = MetadataExtractor::new()
        .extract(&[MetadataBlock::Exif(&exif)])
        .unwrap();
    let enc = MetadataEmbedder::new().emit_iptc_iim(true).embed(&meta).unwrap();
    assert!(enc.iptc_iim.is_none());
}

#[test]
fn embed_emits_legacy_iim_only_on_request() {
    let xmp = xmp_bytes_city("Paris");
    let meta = MetadataExtractor::new()
        .extract(&[MetadataBlock::Xmp(&xmp)])
        .unwrap();

    // Default: IPTC rides inside XMP; no separate legacy block.
    assert!(MetadataEmbedder::new().embed(&meta).unwrap().iptc_iim.is_none());

    // Opt-in: the legacy IIM block is produced (in the chosen charset) and re-parses to the City.
    let enc = MetadataEmbedder::new()
        .emit_iptc_iim(true)
        .iim_charset(IimCharset::Latin1)
        .embed(&meta)
        .unwrap();
    let iim = enc.iptc_iim.expect("legacy IIM emitted on request");
    let reparsed = MetadataExtractor::new()
        .extract(&[MetadataBlock::IptcIim(&iim)])
        .unwrap();
    assert_eq!(reparsed.iptc().unwrap().city(), Some("Paris"));
}

// --- keystone: extract → embed → extract equality ------------------------------------------------

#[test]
fn roundtrip_all_carriers_is_lossless() {
    let (exif, xmp, icc) = (exif_bytes(), xmp_bytes_non_iptc(), icc_bytes());
    let m1 = MetadataExtractor::new()
        .extract(&[
            MetadataBlock::Exif(&exif),
            MetadataBlock::Xmp(&xmp),
            MetadataBlock::Icc(&icc),
        ])
        .unwrap();
    assert!(m1.exif.is_some() && m1.xmp.is_some() && m1.icc.is_some());

    let m2 = reextract(&m1.encode().unwrap());
    assert_eq!(m1, m2);
}

#[test]
fn roundtrip_iim_field_survives_via_xmp_without_reemitting_iim() {
    // Non-IPTC XMP plus an IPTC City that exists *only* in the legacy IIM carrier.
    let xmp = xmp_bytes_non_iptc();
    let iim = iim_bytes_city("Lyon");
    let m1 = MetadataExtractor::new()
        .extract(&[MetadataBlock::Xmp(&xmp), MetadataBlock::IptcIim(&iim)])
        .unwrap();
    // The IIM-only field was folded into the XMP graph.
    assert_eq!(m1.iptc().unwrap().city(), Some("Lyon"));

    // Default embed does NOT re-emit the legacy block...
    let enc = m1.encode().unwrap();
    assert!(enc.iptc_iim.is_none());

    // ...yet the City survives the round-trip, because it now lives in the XMP packet.
    let m2 = reextract(&enc);
    assert_eq!(m1, m2);
    assert_eq!(m2.iptc().unwrap().city(), Some("Lyon"));
}

#[test]
fn roundtrip_empty_is_stable() {
    let m1 = MetadataExtractor::new().extract(&[]).unwrap();
    let m2 = reextract(&m1.encode().unwrap());
    assert_eq!(m1, m2);
    assert!(m2.is_empty());
}
