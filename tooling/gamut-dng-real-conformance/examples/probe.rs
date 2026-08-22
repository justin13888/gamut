//! Reports what `gamut-dng` currently makes of each DNG passed on the command line.
//!
//! This is the tool used to derive a corpus file's `MANIFEST.toml` expectations from measured
//! behaviour rather than guesswork, and to triage a real file that fails. It asserts nothing.
//!
//! ```sh
//! cargo run --manifest-path tooling/gamut-dng-real-conformance/Cargo.toml \
//!     --example probe -- third_party/gamut-dng-samples/apple/iphone-12-pro/IMG_1361.DNG
//! ```

use gamut_dng::{DngDecoder, DngRewrite, SubImageData, deconstruct};

fn main() {
    for path in std::env::args().skip(1) {
        println!("{}", "=".repeat(78));
        println!("{path}");
        let data = match std::fs::read(&path) {
            Ok(d) => d,
            Err(e) => {
                println!("  read: {e}");
                continue;
            }
        };
        println!("  {} bytes", data.len());
        report_structure(&data);
        report_decode(&data);
        report_rewrite(&data);
    }
}

/// Byte-classification and unknown-field inventory.
fn report_structure(data: &[u8]) {
    match deconstruct(data) {
        Ok(report) => {
            println!(
                "  deconstruct   : classified={} unclassified={}B/{} ranges unclaimed={} unread={} conflicts={} oob={}",
                report.segments.is_fully_classified(),
                report.segments.unclassified_bytes(),
                report.segments.unclassified.len(),
                report.segments.unclaimed_reads.len(),
                report.segments.unread_claims.len(),
                report.segments.conflicts.len(),
                report.segments.out_of_bounds.len(),
            );
            println!(
                "  unknown       : tags={} field_types={} anomalies={}",
                report.unknown_tags.len(),
                report.unknown_fields.len(),
                report.anomalies.len(),
            );
            for a in report.anomalies.iter().take(8) {
                println!("      anomaly: {a:?}");
            }
            for u in report.unknown_fields.iter().take(8) {
                println!("      unknown field type: {u:?}");
            }
        }
        Err(e) => println!("  deconstruct   : ERROR {:?} {e}", e.kind()),
    }
}

/// Full decode, or the typed error that stops it.
fn report_decode(data: &[u8]) {
    let decoded = match DngDecoder::new().decode(data) {
        Ok(d) => d,
        Err(e) => {
            println!(
                "  decode        : ERROR kind={:?} msg={:?} origin={:?} detail={:?}",
                e.kind(),
                e.static_message(),
                e.origin(),
                e.detail()
            );
            return;
        }
    };
    let raw = &decoded.raw;
    println!(
        "  decode        : OK {}x{} bits={} spp={} photometry={:?}",
        raw.dimensions().width,
        raw.dimensions().height,
        raw.bits_per_sample(),
        raw.samples_per_pixel(),
        raw.photometry(),
    );
    println!(
        "  version       : {:?} backward={:?} digest_tag={}",
        decoded.dng_version,
        decoded.backward_version,
        decoded.new_raw_image_digest.is_some(),
    );
    match &decoded.profile {
        Some(profile) => println!(
            "  profile       : model={:?} as_shot_neutral={:?}",
            profile.unique_camera_model(),
            profile.as_shot_neutral(),
        ),
        None => println!("  profile       : none (no colour calibration in the file)"),
    }
    println!(
        "  extras        : ifd0={} raw={} exif={} gain_map={} gain_map2={}",
        decoded.ifd0_extra.len(),
        decoded.raw_extra.len(),
        decoded.exif_extra.len(),
        decoded.gain_table_map.is_some(),
        decoded.gain_table_map2.is_some(),
    );
    let raw_tags: Vec<u16> = decoded.raw_extra.iter().map(|t| t.tag).collect();
    println!("      raw_extra tags: {raw_tags:?}");
    println!("  sub_images    : {}", decoded.sub_images.len());
    for s in &decoded.sub_images {
        let payload = match &s.data {
            SubImageData::Undecoded {
                compression,
                chunks,
            } => format!("Undecoded(compression={compression}, {} chunks)", chunks.len()),
            SubImageData::Decoded(v) => format!("Decoded({} samples)", v.len()),
            _ => "Other".to_string(),
        };
        println!(
            "      kind={:?} {}x{} photometric={} bits={} spp={} -> {payload}",
            s.kind, s.dimensions.width, s.dimensions.height, s.photometric, s.bits_per_sample,
            s.samples_per_pixel,
        );
    }
    match raw.to_linear() {
        Ok(l) => println!(
            "  to_linear     : OK {}x{} spp={}",
            l.width, l.height, l.samples_per_pixel
        ),
        Err(e) => println!(
            "  to_linear     : ERROR {:?} {:?}",
            e.kind(),
            e.static_message()
        ),
    }
}

/// The preserving rewrite round-trip.
fn report_rewrite(data: &[u8]) {
    let opened = match DngRewrite::open(data) {
        Ok(r) => r,
        Err(e) => {
            println!(
                "  rewrite open  : ERROR {:?} {:?}",
                e.kind(),
                e.static_message()
            );
            return;
        }
    };
    match opened.write() {
        Ok(out) => println!(
            "  rewrite       : OK {} bytes, maker_note={:?}, classified={:?}",
            out.bytes.len(),
            out.maker_note,
            deconstruct(&out.bytes).map(|r| r.segments.is_fully_classified()),
        ),
        Err(e) => println!(
            "  rewrite write : ERROR {:?} {:?}",
            e.kind(),
            e.static_message()
        ),
    }
}
