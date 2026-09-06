//! C2PA manifest-store carriage in a TIFF-based container (C2PA 2.4 §A.3.6, §18.5.5).
//!
//! C2PA (Coalition for Content Provenance and Authenticity) embeds its manifest store into a
//! TIFF-compatible file — TIFF/EP, DNG, any TIFF-based RAW — "as the data of a tag with ID 52545
//! (decimal) or 0xCD41 (hexadecimal), with a tag type of 7" (§A.3.6). This module is the one
//! place the workspace states that clause: the tag ([`C2PA_MANIFEST_STORE`]), the placement
//! rule over a directory chain, the two-range exclusion set an external signer hashes around,
//! and the read-side locator that recovers those ranges from a file. `gamut-dng` and
//! `gamut-tiff` call it rather than each re-deriving §A.3.6.
//!
//! It is a **locator and placer only**: the store is opaque bytes. Nothing here parses the JUMBF
//! interior, verifies a hash, checks a signature or reaches a verdict — validation belongs to a
//! C2PA validator downstream (`references/c2pa/README.md`).
//!
//! # Placement (§A.3.6)
//!
//! > there shall only be one C2PA Manifest Store for the entire asset — not one per IFD. As
//! > such, the C2PA IFD Entry shall always be located within the **last IFD of the main-IFD
//! > chain**. For TIFF assets containing one main IFD, the C2PA IFD Entry shall be located
//! > within that IFD or be the only entity within a new IFD following the existing one. […] To
//! > support update manifests, the C2PA Manifest Store should be located at the **end of the
//! > file**; this ensures that changes to the size of the C2PA Manifest Store do not impact any
//! > of the other tag offsets.
//!
//! [`locate`] therefore consults only the last directory of the chain, and [`append_store`]
//! lands the store's bytes after everything else in the file, so a store of a different size
//! moves no other offset — the property that makes a reserve-then-sign flow work: a host writes
//! a zero-filled reservation, an external signer computes its hash over the exclusion ranges and
//! overwrites the reservation in place.
//!
//! # Endianness (§A.3.6)
//!
//! > The value of the ByteOrder field in the TIFF header does not govern the endianness of the
//! > embedded C2PA Manifest Store.
//!
//! The store is `UNDEFINED` bytes and is copied verbatim in both directions. Only the entry's
//! `count` and value/offset words follow the file's byte order.
//!
//! # The exclusion set has two ranges (§18.5.5)
//!
//! > When hashing a TIFF-based asset into which the C2PA Manifest will be embedded, the count
//! > field of the C2PA Manifest Store's IFD Entry (representing the length of the JUMBF data)
//! > should be included in the exclusion ranges. This is to support Update Manifests, which
//! > could change the size of the embedded C2PA Manifest Store.
//!
//! The entry sits in a directory body and the store sits elsewhere, so [`C2paExclusions`] is two
//! disjoint ranges, never one. §18.7.3.3 removed general box hash for TIFF, so `c2pa.hash.data`
//! over these exclusions is the only binding a TIFF-based asset has.

use gamut_core::{Error, Result};

use crate::{
    ByteOrder, FieldType, Ifd, IfdReader, Range, RawEntry, RawIfd, ReadAt, Value, Variant,
    align_word,
};

/// The tag carrying the C2PA manifest store: 52545 (`0xCD41`), type `UNDEFINED` (7) — C2PA 2.4
/// §A.3.6.
pub const C2PA_MANIFEST_STORE: u16 = 52545;

/// The smallest store [`append_store`] accepts: a JUMBF box header, 4-byte `LBox` + 4-byte
/// `TBox` (C2PA 2.4 §8.4.2.3's incidental description of the box framing; see
/// `references/c2pa/README.md`). A manifest store is a JUMBF superbox, so nothing shorter can be
/// one, and a reservation shorter than this could never be filled with a valid store.
///
/// Every value this long is also longer than either variant's inline threshold (4 bytes in
/// classic TIFF, 8 in BigTIFF), so an appended store is always out of line and its two exclusion
/// ranges are always disjoint from each other.
pub const MIN_STORE_LEN: usize = 8;

/// The two byte ranges an external signer excludes from a `c2pa.hash.data` hard binding over a
/// TIFF-based asset (C2PA 2.4 §18.5.5): the manifest store itself and the `count` field of its
/// IFD entry.
///
/// Both are absolute offsets into the file the store was located in (or, for an encoder's report,
/// into the bytes that encoder produced). They never overlap: the count field lies inside a
/// directory body, the store outside it (or, for a foreign file whose store packs inline, in the
/// entry's value word, which follows the count field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct C2paExclusions {
    /// The manifest store's bytes: `len` is the entry's `count`, the store being `UNDEFINED`
    /// (one byte per element).
    pub store: Range,
    /// The entry's `count` field: 4 bytes in classic TIFF, 8 in BigTIFF, at offset 4 of the
    /// entry record (after the 2-byte tag and 2-byte type).
    pub count_field: Range,
}

/// Reserves the manifest-store entry in `ifd`, the directory that will be written as the last
/// IFD of the main chain (§A.3.6): a one-byte inline `UNDEFINED` placeholder under
/// [`C2PA_MANIFEST_STORE`].
///
/// The placeholder is what makes the layout final before the store is placed: the entry is in
/// the directory (so every structure after it lands where it will stay) while its value takes
/// no room in the value pool. Once the file is written, [`append_store`] puts the real store at
/// the end of the file and re-points this entry at it.
pub fn reserve_entry(ifd: &mut Ifd) {
    ifd.set(C2PA_MANIFEST_STORE, Value::Undefined(vec![0]));
}

/// The last directory of the main-IFD chain — where §A.3.6 puts the entry.
fn last_ifd<S: ReadAt>(reader: &mut IfdReader<S>) -> Result<RawIfd> {
    let mut last = None;
    for ifd in reader.ifds() {
        last = Some(ifd?);
    }
    last.ok_or_else(|| Error::invalid_input(env!("CARGO_PKG_NAME"), "TIFF: no IFD"))
}

/// The manifest-store entry of `ifd`, if it carries one of the mandated type.
///
/// A tag-52545 entry of any other type is not a manifest store — §A.3.6 fixes the type at 7 —
/// and is reported as absence rather than an error, so a decoder can still surface it as an
/// unmodelled field. Both sides of the workspace's codecs apply this same test.
fn store_entry(ifd: &RawIfd) -> Option<&RawEntry> {
    ifd.entry(C2PA_MANIFEST_STORE)
        .filter(|entry| entry.field_type() == Some(FieldType::Undefined))
}

/// The count field of `entry`: the offset-width word after the 2-byte tag and 2-byte type.
fn count_field(entry: &RawEntry, variant: Variant) -> Range {
    Range {
        start: entry.offset + 4,
        len: variant.offset_size() as u64,
    }
}

/// Whether a file of `end` bytes has outgrown `variant`'s offset width — possible only for
/// classic TIFF's 32-bit offsets and counts. A pure predicate so the 4 GiB boundary is testable
/// without a 4 GiB allocation.
fn exceeds_offset_width(variant: Variant, end: u64) -> bool {
    variant == Variant::Classic && end > u64::from(u32::MAX)
}

/// Locates the C2PA manifest store of a TIFF-based file and reports its exclusion ranges.
///
/// Walks the main-IFD chain to its **last** directory (§A.3.6) and looks there — and only
/// there — for an `UNDEFINED` entry under [`C2PA_MANIFEST_STORE`]. Returns `Ok(None)` when the
/// last directory has no such entry, when its type is not 7, or (a file that breaks §A.3.6) when
/// the entry sits in an earlier directory of the chain. A store whose value packs inline is
/// reported with `store` inside the entry's value word, after the count field.
///
/// `src` is any [`ReadAt`] source — a `&[u8]`, or a [`StreamSource`](crate::StreamSource) over
/// a file handle, since a multi-hundred-MB RAW need not be read to find a 12-byte entry.
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if the container is unreadable (bad header, looping or
/// runaway chain, no IFD) or the store's declared extent lies outside the source — the same
/// verdicts [`read`](crate::read) gives such a file — or [`Error::Io`] if the source fails.
pub fn locate<S: ReadAt>(src: S) -> Result<Option<C2paExclusions>> {
    let mut reader = IfdReader::open(src)?;
    let last = last_ifd(&mut reader)?;
    let Some(entry) = store_entry(&last) else {
        return Ok(None);
    };
    let variant = reader.variant();
    let count_field = count_field(entry, variant);
    // Out of line at the offset the entry declares; inline, the value word follows the count.
    let start = reader.value_offset(entry).unwrap_or(count_field.end());
    let end = start.checked_add(entry.count).ok_or_else(|| {
        Error::invalid_input(
            env!("CARGO_PKG_NAME"),
            "TIFF: C2PA manifest store extent overflows",
        )
    })?;
    if end > reader.source_mut().len()? {
        return Err(Error::invalid_input(
            env!("CARGO_PKG_NAME"),
            "TIFF: C2PA manifest store lies outside the file",
        ));
    }
    Ok(Some(C2paExclusions {
        store: Range {
            start,
            len: entry.count,
        },
        count_field,
    }))
}

/// Appends `store` at the (word-aligned) end of `file` and points the reserved entry at it,
/// returning the exclusion ranges.
///
/// `file` is a complete TIFF-based file whose last main-chain IFD carries the entry
/// [`reserve_entry`] placed — a one-byte inline placeholder. The store lands at
/// [`align_word`]`(file.len())`, with one zero byte of alignment filler when the file was of odd
/// length, and the entry's `count` and value/offset words are rewritten in the file's byte order.
/// Nothing else moves: every other offset in the file is untouched, which is what lets a
/// zero-filled reservation of the same length be overwritten in place by an external signer.
/// The bytes of `store` are copied verbatim — the TIFF byte order does not apply to them
/// (§A.3.6).
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if `store` is shorter than [`MIN_STORE_LEN`]; if the
/// container is unreadable; if its last IFD carries no [`C2PA_MANIFEST_STORE`] entry of type
/// `UNDEFINED`; if that entry already points out of line (re-pointing it would orphan the bytes
/// it points at — write a placeholder instead); or if the appended store would put the file past
/// the 4 GiB classic-TIFF offset limit.
pub fn append_store(file: &mut Vec<u8>, store: &[u8]) -> Result<C2paExclusions> {
    if store.len() < MIN_STORE_LEN {
        return Err(Error::invalid_input(
            env!("CARGO_PKG_NAME"),
            "TIFF: C2PA manifest store is shorter than a JUMBF box header",
        ));
    }
    let (order, variant, entry) = {
        let mut reader = IfdReader::open(&file[..])?;
        let last = last_ifd(&mut reader)?;
        let entry = store_entry(&last)
            .ok_or_else(|| {
                Error::invalid_input(
                    env!("CARGO_PKG_NAME"),
                    "TIFF: the last IFD carries no reserved C2PA manifest store entry",
                )
            })?
            .clone();
        if reader.value_offset(&entry).is_some() {
            return Err(Error::invalid_input(
                env!("CARGO_PKG_NAME"),
                "TIFF: the C2PA manifest store entry already points out of line",
            ));
        }
        (reader.order(), reader.variant(), entry)
    };

    let start = align_word(file.len() as u64);
    let len = store.len() as u64;
    let end = start.checked_add(len).ok_or_else(|| {
        Error::invalid_input(
            env!("CARGO_PKG_NAME"),
            "TIFF: C2PA manifest store extent overflows",
        )
    })?;
    if exceeds_offset_width(variant, end) {
        return Err(Error::invalid_input(
            env!("CARGO_PKG_NAME"),
            "TIFF: C2PA manifest store exceeds the 4 GiB classic-TIFF offset limit",
        ));
    }
    let start_usize = usize::try_from(start)
        .map_err(|_| Error::invalid_input(env!("CARGO_PKG_NAME"), "TIFF: layout overflows"))?;
    file.resize(start_usize, 0);
    file.extend_from_slice(store);

    let count_field = count_field(&entry, variant);
    patch_entry(file, &count_field, len, start, order, variant);
    Ok(C2paExclusions {
        store: Range { start, len },
        count_field,
    })
}

/// Rewrites the entry's `count` and value/offset words — the offset-width pair after the tag
/// and type — in the file's byte order.
fn patch_entry(
    file: &mut [u8],
    count_field: &Range,
    count: u64,
    offset: u64,
    order: ByteOrder,
    variant: Variant,
) {
    let at = count_field.start as usize;
    match variant {
        Variant::Classic => {
            file[at..at + 4].copy_from_slice(&order.pack_u32(count as u32));
            file[at + 4..at + 8].copy_from_slice(&order.pack_u32(offset as u32));
        }
        #[cfg(feature = "bigtiff")]
        Variant::Big => {
            file[at..at + 8].copy_from_slice(&order.pack_u64(count));
            file[at + 8..at + 16].copy_from_slice(&order.pack_u64(offset));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Segment, SegmentReport, SpanKind, TiffFile, WriteOptions, read, read_audited, read_header,
        write, write_with,
    };

    /// A store whose bytes are not a palindrome and not repeated, so a byte-swapped or
    /// misaligned copy cannot equal it.
    fn store() -> Vec<u8> {
        (0x10u8..0x30).collect()
    }

    /// Directory fields around the store: an inline value below the tag, and out-of-line values
    /// on both sides of it, so the store's value is neither first nor last in the pool.
    fn sample_ifd() -> Ifd {
        let mut ifd = Ifd::new();
        ifd.set(256, Value::Short(vec![640]));
        ifd.set(258, Value::Short(vec![8, 8, 8])); // 6 bytes: out of line in classic TIFF
        ifd.set(65000, Value::Undefined(vec![0xEE; 5])); // odd length, past the store's tag
        ifd
    }

    fn file(order: ByteOrder, variant: Variant, ifds: Vec<Ifd>) -> TiffFile {
        TiffFile {
            order,
            variant,
            ifds,
        }
    }

    /// The writer's own declaration of where the `Value { tag }` span of the sole directory
    /// landed.
    fn declared_value_span(report: &SegmentReport, tag: u16) -> Range {
        report
            .segments
            .iter()
            .find(|s| matches!(s.kind, SpanKind::Value { tag: t, .. } if t == tag))
            .map(|s| s.range)
            .expect("the writer declares the value span")
    }

    fn ifd_body_offset(report: &SegmentReport) -> u64 {
        report
            .segments
            .iter()
            .find_map(|s| match s.kind {
                SpanKind::IfdBody { ifd } => Some(ifd),
                _ => None,
            })
            .expect("one directory body")
    }

    /// `locate` reports exactly the span the writer put the store in, and the count field at
    /// its hand-computed position in the entry — pinned in both byte orders by reading the
    /// count back through the file's own byte order.
    #[test]
    fn locate_reports_the_store_span_and_the_count_field() {
        for order in [ByteOrder::LittleEndian, ByteOrder::BigEndian] {
            let mut ifd = sample_ifd();
            ifd.set(C2PA_MANIFEST_STORE, Value::Undefined(store()));
            let (bytes, map) = write_with(
                &file(order, Variant::Classic, vec![ifd]),
                &WriteOptions::default(),
            )
            .expect("write");
            let report = map.finish(None);

            let found = locate(&bytes[..]).expect("locate").expect("a store");
            assert_eq!(
                found.store,
                declared_value_span(&report, C2PA_MANIFEST_STORE)
            );
            // Tags sort 256, 258, 52545, 65000: the store is entry 2 of the directory. Count
            // field = body + 2 (entry count) + 2 * 12 + 4 (tag, type).
            let body = ifd_body_offset(&report);
            assert_eq!(
                found.count_field,
                Range {
                    start: body + 2 + 2 * 12 + 4,
                    len: 4
                },
                "{order:?}"
            );
            let at = found.count_field.start as usize;
            let count = order.u32(bytes[at..at + 4].try_into().expect("4 bytes"));
            assert_eq!(u64::from(count), found.store.len, "{order:?}");
            assert_eq!(
                &bytes[found.store.start as usize..found.store.end() as usize],
                store().as_slice(),
                "{order:?}: the store's bytes are verbatim, not byte-swapped"
            );
        }
    }

    /// A BigTIFF count field is 8 bytes wide and the store is still found by its declared
    /// offset.
    #[cfg(feature = "bigtiff")]
    #[test]
    fn locate_reads_bigtiff_count_and_offset_widths() {
        let mut ifd = sample_ifd();
        ifd.set(C2PA_MANIFEST_STORE, Value::Undefined(store()));
        let (bytes, map) = write_with(
            &file(ByteOrder::BigEndian, Variant::Big, vec![ifd]),
            &WriteOptions::default(),
        )
        .expect("write");
        let report = map.finish(None);
        let found = locate(&bytes[..]).expect("locate").expect("a store");
        assert_eq!(
            found.store,
            declared_value_span(&report, C2PA_MANIFEST_STORE)
        );
        let body = ifd_body_offset(&report);
        assert_eq!(
            found.count_field,
            Range {
                start: body + 8 + 2 * 20 + 4,
                len: 8
            }
        );
    }

    /// No entry, an entry of the wrong type, and an entry in a directory that is not the last of
    /// the chain are all "no store" — not errors.
    #[test]
    fn locate_reports_absence_for_a_missing_mistyped_or_misplaced_entry() {
        let plain = write(&file(
            ByteOrder::LittleEndian,
            Variant::Classic,
            vec![sample_ifd()],
        ))
        .expect("write");
        assert_eq!(locate(&plain[..]).expect("locate"), None);

        let mut mistyped = sample_ifd();
        mistyped.set(C2PA_MANIFEST_STORE, Value::Byte(store()));
        let bytes = write(&file(
            ByteOrder::LittleEndian,
            Variant::Classic,
            vec![mistyped],
        ))
        .expect("write");
        assert_eq!(locate(&bytes[..]).expect("locate"), None);

        // The entry in IFD 0 of a two-directory chain: §A.3.6 wants it in the last one.
        let mut first = sample_ifd();
        first.set(C2PA_MANIFEST_STORE, Value::Undefined(store()));
        let bytes = write(&file(
            ByteOrder::LittleEndian,
            Variant::Classic,
            vec![first, sample_ifd()],
        ))
        .expect("write");
        assert_eq!(locate(&bytes[..]).expect("locate"), None);
    }

    /// The entry in the last directory of a two-directory chain — the "new IFD following the
    /// existing one" form §A.3.6 allows — is found there.
    #[test]
    fn locate_follows_the_chain_to_its_last_directory() {
        let mut last = Ifd::new();
        last.set(C2PA_MANIFEST_STORE, Value::Undefined(store()));
        let (bytes, map) = write_with(
            &file(
                ByteOrder::LittleEndian,
                Variant::Classic,
                vec![sample_ifd(), last],
            ),
            &WriteOptions::default(),
        )
        .expect("write");
        let report = map.finish(None);
        let found = locate(&bytes[..]).expect("locate").expect("a store");
        assert_eq!(
            found.store,
            declared_value_span(&report, C2PA_MANIFEST_STORE)
        );
        // The second directory is the one whose body holds the count field.
        let second_body = report
            .segments
            .iter()
            .filter_map(|s| match s.kind {
                SpanKind::IfdBody { ifd } => Some(ifd),
                _ => None,
            })
            .max()
            .expect("two bodies");
        assert_eq!(found.count_field.start, second_body + 2 + 4);
    }

    /// A store short enough to pack inline is reported inside its entry's value word, after the
    /// count field.
    #[test]
    fn locate_reports_an_inline_store_inside_its_entry() {
        let mut ifd = Ifd::new();
        ifd.set(
            C2PA_MANIFEST_STORE,
            Value::Undefined(vec![0xA1, 0xB2, 0xC3]),
        );
        let bytes =
            write(&file(ByteOrder::LittleEndian, Variant::Classic, vec![ifd])).expect("write");
        let found = locate(&bytes[..]).expect("locate").expect("a store");
        // Header 8, entry count 2: the entry starts at 10, its count at 14, its value at 18.
        assert_eq!(found.count_field, Range { start: 14, len: 4 });
        assert_eq!(found.store, Range { start: 18, len: 3 });
        assert_eq!(&bytes[18..21], &[0xA1, 0xB2, 0xC3]);
    }

    /// A store whose declared extent runs past the end of the file is a typed error, not a range
    /// a signer could hash.
    #[test]
    fn locate_rejects_a_store_past_the_end_of_file() {
        let mut ifd = Ifd::new();
        ifd.set(C2PA_MANIFEST_STORE, Value::Undefined(store()));
        let mut bytes =
            write(&file(ByteOrder::LittleEndian, Variant::Classic, vec![ifd])).expect("write");
        // Point the entry's offset word (at 18) far past the end.
        bytes[18..22].copy_from_slice(&ByteOrder::LittleEndian.pack_u32(100_000));
        let error = locate(&bytes[..]).expect_err("out of bounds");
        assert_eq!(
            error.static_message(),
            Some("TIFF: C2PA manifest store lies outside the file")
        );
        // Exactly at the end is in bounds: the check is `>`, not `>=`.
        let mut ifd = Ifd::new();
        ifd.set(C2PA_MANIFEST_STORE, Value::Undefined(store()));
        let bytes =
            write(&file(ByteOrder::LittleEndian, Variant::Classic, vec![ifd])).expect("write");
        let found = locate(&bytes[..]).expect("locate").expect("a store");
        assert_eq!(found.store.end(), bytes.len() as u64);
    }

    /// A chain with no directory at all is the container-level error `read` gives it.
    #[test]
    fn locate_rejects_an_empty_chain() {
        let error = locate(&b"II\x2a\x00\x00\x00\x00\x00"[..]).expect_err("no IFD");
        assert_eq!(error.static_message(), Some("TIFF: no IFD"));
    }

    /// The write-side flow: reserve, write, append. The store lands last, verbatim, at the
    /// word-aligned end (one zero filler byte after an odd-length file); the entry reads back
    /// as the store; the reported ranges are what `locate` finds; and an independent audit of
    /// the result classifies every byte, with the store as the entry's value span and the
    /// filler as padding — never a trailer.
    #[test]
    fn append_store_lands_the_store_last_and_repoints_the_entry() {
        for order in [ByteOrder::LittleEndian, ByteOrder::BigEndian] {
            let mut ifd = sample_ifd();
            reserve_entry(&mut ifd);
            let mut bytes = write(&file(order, Variant::Classic, vec![ifd])).expect("write");
            // `sample_ifd`'s 5-byte value is last in the pool, so the file ends odd.
            assert_eq!(
                bytes.len() % 2,
                1,
                "fixture must end odd to exercise alignment"
            );
            let before = bytes.clone();

            let excl = append_store(&mut bytes, &store()).expect("append");
            let start = align_word(before.len() as u64);
            assert_eq!(
                excl.store,
                Range {
                    start,
                    len: store().len() as u64
                }
            );
            assert_eq!(bytes.len() as u64, excl.store.end(), "the store is last");
            assert_eq!(bytes[before.len()], 0, "one zero byte of alignment filler");
            assert_eq!(
                &bytes[start as usize..],
                store().as_slice(),
                "{order:?}: verbatim"
            );
            // Nothing before the filler changed except the entry's count and offset words.
            let cf = excl.count_field.start as usize;
            assert_eq!(&bytes[..cf], &before[..cf]);
            assert_eq!(&bytes[cf + 8..before.len()], &before[cf + 8..]);
            assert_eq!(
                order.u32(bytes[cf..cf + 4].try_into().expect("4")),
                store().len() as u32
            );
            assert_eq!(
                u64::from(order.u32(bytes[cf + 4..cf + 8].try_into().expect("4"))),
                start
            );

            let parsed = read(&bytes).expect("read back");
            assert_eq!(
                parsed.ifds[0].get(C2PA_MANIFEST_STORE),
                Some(&Value::Undefined(store()))
            );
            assert_eq!(locate(&bytes[..]).expect("locate"), Some(excl));

            let (_, mut report) = read_audited(&bytes).expect("audit");
            report
                .classify_padding(&mut (&bytes[..]))
                .expect("classify");
            assert!(report.is_fully_classified(), "{order:?}: {report:?}");
            let (_, _, ifd0) = read_header(&bytes).expect("header");
            assert!(report.segments.contains(&Segment {
                range: excl.store,
                kind: SpanKind::Value {
                    ifd: ifd0,
                    tag: C2PA_MANIFEST_STORE
                },
            }));
            assert!(report.segments.contains(&Segment {
                range: Range {
                    start: before.len() as u64,
                    len: 1
                },
                kind: SpanKind::Padding,
            }));
        }
    }

    /// An even-length file needs no filler: the store starts exactly where the file ended.
    #[test]
    fn append_store_adds_no_filler_to_an_even_length_file() {
        let mut ifd = Ifd::new();
        ifd.set(256, Value::Short(vec![640]));
        reserve_entry(&mut ifd);
        let mut bytes =
            write(&file(ByteOrder::LittleEndian, Variant::Classic, vec![ifd])).expect("write");
        assert_eq!(bytes.len() % 2, 0);
        let len = bytes.len() as u64;
        let excl = append_store(&mut bytes, &store()).expect("append");
        assert_eq!(excl.store.start, len);
    }

    /// BigTIFF: 8-byte count and offset words are rewritten, and the store reads back.
    #[cfg(feature = "bigtiff")]
    #[test]
    fn append_store_rewrites_bigtiff_words() {
        let mut ifd = sample_ifd();
        reserve_entry(&mut ifd);
        let mut bytes = write(&file(ByteOrder::BigEndian, Variant::Big, vec![ifd])).expect("write");
        let excl = append_store(&mut bytes, &store()).expect("append");
        assert_eq!(excl.count_field.len, 8);
        let cf = excl.count_field.start as usize;
        assert_eq!(
            ByteOrder::BigEndian.u64(bytes[cf..cf + 8].try_into().expect("8")),
            store().len() as u64
        );
        assert_eq!(
            ByteOrder::BigEndian.u64(bytes[cf + 8..cf + 16].try_into().expect("8")),
            excl.store.start
        );
        assert_eq!(
            read(&bytes).expect("read").ifds[0].get(C2PA_MANIFEST_STORE),
            Some(&Value::Undefined(store()))
        );
        assert_eq!(locate(&bytes[..]).expect("locate"), Some(excl));
    }

    /// Each precondition failure is its own typed error: a short store, no reserved entry, a
    /// mistyped entry, and an entry already pointing out of line.
    #[test]
    fn append_store_refuses_unsatisfiable_inputs() {
        let le = |ifd| {
            write(&file(ByteOrder::LittleEndian, Variant::Classic, vec![ifd])).expect("write")
        };
        let mut reserved = sample_ifd();
        reserve_entry(&mut reserved);

        let error = append_store(&mut le(reserved.clone()), &store()[..7]).expect_err("short");
        assert_eq!(
            error.static_message(),
            Some("TIFF: C2PA manifest store is shorter than a JUMBF box header")
        );
        // Exactly the minimum is accepted.
        assert!(append_store(&mut le(reserved.clone()), &store()[..8]).is_ok());

        let error = append_store(&mut le(sample_ifd()), &store()).expect_err("no entry");
        assert_eq!(
            error.static_message(),
            Some("TIFF: the last IFD carries no reserved C2PA manifest store entry")
        );

        let mut mistyped = sample_ifd();
        mistyped.set(C2PA_MANIFEST_STORE, Value::Byte(vec![0]));
        let error = append_store(&mut le(mistyped), &store()).expect_err("mistyped");
        assert_eq!(
            error.static_message(),
            Some("TIFF: the last IFD carries no reserved C2PA manifest store entry")
        );

        let mut out_of_line = sample_ifd();
        out_of_line.set(C2PA_MANIFEST_STORE, Value::Undefined(store()));
        let error = append_store(&mut le(out_of_line), &store()).expect_err("out of line");
        assert_eq!(
            error.static_message(),
            Some("TIFF: the C2PA manifest store entry already points out of line")
        );
    }

    /// The 4 GiB guard's boundary on the pure predicate (a >4 GiB allocation is not testable):
    /// the largest classic offset is representable, one past it is not, BigTIFF never overflows.
    #[test]
    fn offset_width_boundary() {
        assert!(!exceeds_offset_width(Variant::Classic, 100));
        assert!(!exceeds_offset_width(Variant::Classic, u64::from(u32::MAX)));
        assert!(exceeds_offset_width(
            Variant::Classic,
            u64::from(u32::MAX) + 1
        ));
        #[cfg(feature = "bigtiff")]
        assert!(!exceeds_offset_width(Variant::Big, u64::from(u32::MAX) + 1));
    }

    /// `reserve_entry` places exactly the inline placeholder `append_store` requires.
    #[test]
    fn reserve_entry_places_a_one_byte_inline_placeholder() {
        let mut ifd = Ifd::new();
        reserve_entry(&mut ifd);
        assert_eq!(
            ifd.get(C2PA_MANIFEST_STORE),
            Some(&Value::Undefined(vec![0]))
        );
        let bytes =
            write(&file(ByteOrder::LittleEndian, Variant::Classic, vec![ifd])).expect("write");
        let found = locate(&bytes[..]).expect("locate").expect("placeholder");
        assert_eq!(found.store.len, 1);
        assert_eq!(
            found.store.start,
            found.count_field.end(),
            "inline: in the value word"
        );
    }
}
