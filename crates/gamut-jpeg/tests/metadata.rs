//! Integration tests for the APP1/APP2 metadata surface: `metadata()` extraction of EXIF, XMP, and
//! multi-segment ICC payloads, the ICC chunk-consistency error corpus, and the skip rules for
//! foreign or continuation segments.

use gamut_core::{Dimensions, EncodeImage, Error, Gray8, ImageRef};
use gamut_jpeg::{JpegEncoder, JpegMetadata, metadata};

/// Encodes a minimal valid grayscale JPEG to splice APP segments into.
fn base_jpeg() -> Vec<u8> {
    let pixels = vec![128u8; 64];
    let image = ImageRef::<Gray8>::new(&pixels, Dimensions::new(8, 8).unwrap()).unwrap();
    JpegEncoder::new().encode_to_vec(image).unwrap()
}

/// Builds an APPn marker segment: `0xFF, code`, big-endian length (counting itself), payload.
fn app_segment(code: u8, payload: &[u8]) -> Vec<u8> {
    let mut seg = vec![0xFF, code];
    let len = u16::try_from(payload.len() + 2).unwrap();
    seg.extend_from_slice(&len.to_be_bytes());
    seg.extend_from_slice(payload);
    seg
}

/// Inserts segments immediately after the SOI marker of `jpeg`.
fn splice(jpeg: &[u8], segments: &[Vec<u8>]) -> Vec<u8> {
    let mut out = jpeg[..2].to_vec();
    for seg in segments {
        out.extend_from_slice(seg);
    }
    out.extend_from_slice(&jpeg[2..]);
    out
}

/// Builds one APP2 `ICC_PROFILE` payload (signature, 1-based index, total count, chunk data).
fn icc_payload(index: u8, count: u8, data: &[u8]) -> Vec<u8> {
    let mut payload = b"ICC_PROFILE\0".to_vec();
    payload.push(index);
    payload.push(count);
    payload.extend_from_slice(data);
    payload
}

#[test]
fn plain_stream_has_no_metadata() {
    assert_eq!(metadata(&base_jpeg()).unwrap(), JpegMetadata::default());
}

#[test]
fn exif_app1_is_extracted_with_the_signature_stripped() {
    // A tiny but shaped TIFF stream: little-endian header + zero-entry IFD.
    let tiff = b"II\x2A\x00\x08\x00\x00\x00\x00\x00".to_vec();
    let mut payload = b"Exif\0\0".to_vec();
    payload.extend_from_slice(&tiff);
    let jpeg = splice(&base_jpeg(), &[app_segment(0xE1, &payload)]);
    let meta = metadata(&jpeg).unwrap();
    assert_eq!(meta.exif.as_deref(), Some(tiff.as_slice()));
    assert_eq!(meta.xmp, None);
    assert_eq!(meta.icc, None);
}

#[test]
fn xmp_app1_is_extracted_with_the_uri_stripped() {
    let packet = b"<?xpacket begin=\"\"?><x:xmpmeta/><?xpacket end=\"w\"?>".to_vec();
    let mut payload = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
    payload.extend_from_slice(&packet);
    let jpeg = splice(&base_jpeg(), &[app_segment(0xE1, &payload)]);
    assert_eq!(
        metadata(&jpeg).unwrap().xmp.as_deref(),
        Some(packet.as_slice())
    );
}

#[test]
fn single_chunk_icc_is_extracted() {
    let profile: Vec<u8> = (0..200u32).map(|i| (i % 251) as u8).collect();
    let jpeg = splice(
        &base_jpeg(),
        &[app_segment(0xE2, &icc_payload(1, 1, &profile))],
    );
    assert_eq!(
        metadata(&jpeg).unwrap().icc.as_deref(),
        Some(profile.as_slice())
    );
}

#[test]
fn out_of_order_icc_chunks_reassemble_by_index() {
    // Three distinct chunks delivered 2, 3, 1: reassembly must follow the index bytes, not the
    // segment order.
    let (a, b, c) = (vec![1u8; 5], vec![2u8; 7], vec![3u8; 3]);
    let jpeg = splice(
        &base_jpeg(),
        &[
            app_segment(0xE2, &icc_payload(2, 3, &b)),
            app_segment(0xE2, &icc_payload(3, 3, &c)),
            app_segment(0xE2, &icc_payload(1, 3, &a)),
        ],
    );
    let expected: Vec<u8> = [a, b, c].concat();
    assert_eq!(metadata(&jpeg).unwrap().icc, Some(expected));
}

#[test]
fn icc_chunk_inconsistencies_are_rejected() {
    let base = base_jpeg();
    let cases: &[&[Vec<u8>]] = &[
        // Count disagrees between chunks.
        &[
            app_segment(0xE2, &icc_payload(1, 2, b"aa")),
            app_segment(0xE2, &icc_payload(2, 3, b"bb")),
        ],
        // Index 0 (indices are 1-based).
        &[app_segment(0xE2, &icc_payload(0, 1, b"aa"))],
        // Zero count.
        &[app_segment(0xE2, &icc_payload(1, 0, b"aa"))],
        // Index beyond the count.
        &[app_segment(0xE2, &icc_payload(3, 2, b"aa"))],
        // Duplicated index.
        &[
            app_segment(0xE2, &icc_payload(1, 2, b"aa")),
            app_segment(0xE2, &icc_payload(1, 2, b"bb")),
        ],
        // Declared 2 chunks, delivered 1.
        &[app_segment(0xE2, &icc_payload(1, 2, b"aa"))],
        // Signature present but the index/count bytes are missing.
        &[app_segment(0xE2, b"ICC_PROFILE\0")],
    ];
    for segments in cases {
        let jpeg = splice(&base, segments);
        assert!(
            matches!(metadata(&jpeg), Err(Error::InvalidInput(_))),
            "case {segments:?} was not rejected"
        );
    }
}

#[test]
fn foreign_app_segments_are_skipped() {
    // A non-ICC APP2 (Exif Flashpix "FPXR"), an ExtendedXMP APP1 continuation, and an unrelated
    // APP1 must all be ignored without error.
    let jpeg = splice(
        &base_jpeg(),
        &[
            app_segment(0xE2, b"FPXR\0not an icc chunk"),
            app_segment(
                0xE1,
                b"http://ns.adobe.com/xmp/extension/\0guid-and-chunk-data",
            ),
            app_segment(0xE1, b"not a known app1 payload"),
        ],
    );
    assert_eq!(metadata(&jpeg).unwrap(), JpegMetadata::default());
}

#[test]
fn duplicate_exif_and_xmp_app1_first_wins() {
    let first = b"Exif\0\0II\x2A\x00first".to_vec();
    let second = b"Exif\0\0II\x2A\x00second".to_vec();
    let xmp_first = b"http://ns.adobe.com/xap/1.0/\0<first/>".to_vec();
    let xmp_second = b"http://ns.adobe.com/xap/1.0/\0<second/>".to_vec();
    let jpeg = splice(
        &base_jpeg(),
        &[
            app_segment(0xE1, &first),
            app_segment(0xE1, &xmp_first),
            app_segment(0xE1, &second),
            app_segment(0xE1, &xmp_second),
        ],
    );
    let meta = metadata(&jpeg).unwrap();
    assert_eq!(meta.exif.as_deref(), Some(&b"II\x2A\x00first"[..]));
    assert_eq!(meta.xmp.as_deref(), Some(&b"<first/>"[..]));
}

#[test]
fn metadata_stops_at_the_scan() {
    // An APP1 spliced after the SOS marker is entropy data territory; metadata() must have stopped.
    let base = base_jpeg();
    let sos = base
        .windows(2)
        .position(|w| w == [0xFF, 0xDA])
        .expect("stream has an SOS");
    let mut payload = b"Exif\0\0".to_vec();
    payload.extend_from_slice(b"II\x2A\x00");
    let seg = app_segment(0xE1, &payload);
    let mut jpeg = base[..sos].to_vec();
    jpeg.extend_from_slice(&seg);
    jpeg.extend_from_slice(&base[sos..]);
    // The segment sits *before* SOS here, so it IS found …
    assert!(metadata(&jpeg).unwrap().exif.is_some());
    // … but appended after the EOI-terminated scan it is not reachable: truncate the walk at SOS.
    let mut after = base.clone();
    after.extend_from_slice(&seg);
    assert_eq!(metadata(&after).unwrap(), JpegMetadata::default());
}

#[test]
fn malformed_streams_are_rejected() {
    // Missing SOI.
    assert!(matches!(
        metadata(b"\xFF\xE1\x00\x04ab"),
        Err(Error::InvalidInput(_))
    ));
    // A standalone marker (RST0) where a segment is expected.
    assert!(matches!(
        metadata(&[0xFF, 0xD8, 0xFF, 0xD0]),
        Err(Error::InvalidInput(_))
    ));
    // A declared segment length running past the end of the data.
    let truncated = [0xFF, 0xD8, 0xFF, 0xE1, 0xFF, 0xFF, 0x00];
    assert!(matches!(metadata(&truncated), Err(Error::InvalidInput(_))));
}
