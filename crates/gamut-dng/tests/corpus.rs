//! Byte-completeness over the Adobe DNG SDK's own sample corpus (issue #263): fourteen
//! Adobe-authored DNGs exercising features far beyond this crate's codec scope (JPEG XL tiles,
//! ProfileGainTableMap, ImageStats, ImageSequenceInfo, HDR/SDR profiles). The deconstruct must
//! account these files it cannot *decode*: the dual-ledger parser invariants must hold
//! unconditionally, and every byte must be classified.

use gamut_core::Error;
use gamut_dng::{DngRewrite, deconstruct};

#[test]
fn adobe_sample_corpus_is_fully_classified() {
    let dir = gamut_dng_oracle::sample_files_dir();
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .expect("sample_files extracted by the oracle build")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "dng"))
        .collect();
    paths.sort();
    assert!(
        paths.len() >= 14,
        "expected the SDK's sample corpus, found {paths:?}"
    );

    for path in &paths {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let data = std::fs::read(path).expect("read sample");
        let report = deconstruct(&data).unwrap_or_else(|e| panic!("{name}: deconstruct: {e}"));

        // The dual-ledger invariants are unconditional: any violation is a gamut parser bug,
        // never a property of the file.
        assert!(
            report.segments.unclaimed_reads.is_empty(),
            "{name}: parser read bytes it never claimed: {:?}",
            report.segments.unclaimed_reads
        );
        assert!(
            report.segments.unread_claims.is_empty(),
            "{name}: parser claimed bytes it never read: {:?}",
            report.segments.unread_claims
        );

        // Byte completeness: every byte of every Adobe-authored sample maps to a typed segment
        // (structure, value, data extent, or classified padding) — the issue #263 verification.
        assert!(
            report.segments.is_fully_classified(),
            "{name}: {} unclassified byte(s) in {} range(s); conflicts {:?}; oob {:?}",
            report.segments.unclassified_bytes(),
            report.segments.unclassified.len(),
            report.segments.conflicts,
            report.segments.out_of_bounds,
        );

        // These files are pure spec-tag DNGs: nothing should surface as an unknown *field
        // type*. (Unknown tags/codes are allowed — several samples carry DNG-1.7 features this
        // crate does not model — and are exactly what the report is for.)
        assert!(
            report.unknown_fields.is_empty(),
            "{name}: unknown field types: {:?}",
            report.unknown_fields
        );
    }
}

/// The preserving rewrite over the real corpus: every rewritable Adobe sample survives an
/// open → write round-trip with the Adobe SDK still accepting it, the rewrite classifying
/// fully, and the unknown-tag inventory unchanged (nothing dropped).
#[test]
fn adobe_sample_corpus_survives_a_preserving_rewrite() {
    let dir = gamut_dng_oracle::sample_files_dir();
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .expect("sample_files extracted by the oracle build")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "dng"))
        .collect();
    paths.sort();

    let mut rewritten = 0usize;
    for path in &paths {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let data = std::fs::read(path).expect("read sample");
        let rewrite = match DngRewrite::open(&data) {
            Ok(r) => r,
            // Embedded camera-profile carriage through a rewrite is explicitly deferred.
            Err(Error::Unsupported(_)) => continue,
            Err(e) => panic!("{name}: open: {e}"),
        };
        let out = rewrite
            .write()
            .unwrap_or_else(|e| panic!("{name}: write: {e}"));

        let before = deconstruct(&data).expect("deconstruct original");
        let after = deconstruct(&out.bytes).unwrap_or_else(|e| panic!("{name}: deconstruct: {e}"));
        assert!(
            after.segments.is_fully_classified(),
            "{name}: rewrite not fully classified: {:?}",
            after.segments.unclassified
        );
        let tags = |r: &gamut_dng::DeconstructReport| {
            r.unknown_tags.iter().map(|u| u.tag).collect::<Vec<_>>()
        };
        assert_eq!(
            tags(&after),
            tags(&before),
            "{name}: unknown-tag inventory changed across the rewrite"
        );
        // The SDK gate applies wherever the SDK accepts the *original* (the oracle build stubs
        // libjxl, so it rejects the JXL-compressed samples themselves — original and rewrite
        // alike; the structural comparison above still covers them).
        if gamut_dng_oracle::validate_dng(&data).is_ok() {
            gamut_dng_oracle::validate_dng(&out.bytes)
                .unwrap_or_else(|e| panic!("{name}: Adobe SDK rejected the rewrite: {e}"));
        }
        rewritten += 1;
    }
    assert!(
        rewritten >= 10,
        "most of the corpus must be rewritable ({rewritten} were)"
    );
}
