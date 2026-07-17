//! Streaming-vs-slice equivalence: the [`IfdReader`] must produce exactly what the eager slice
//! readers produce, through every source shape — a borrowed slice, a seekable stream, and an
//! offset-rebased view — and must honor its laziness contract (only the structure actually
//! asked for is read).

use gamut_core::Result;
use gamut_ifd::{
    ByteOrder, Coverage, Ifd, IfdReader, ReadAt, StreamSource, TiffFile, Value, Variant, read,
    read_tree, read_with_coverage, tags, write,
};

/// A flat single-IFD file, a two-IFD chain, and a nested sub-IFD tree — the shapes the
/// fixtures cover for every order/variant.
fn fixture_files(order: ByteOrder, variant: Variant) -> Vec<TiffFile> {
    let mut flat = Ifd::new();
    flat.set(256, Value::Short(vec![640]));
    flat.set(270, Value::Ascii("first\0second".to_owned())); // multi-string, out of line
    flat.set(282, Value::Rational(vec![(300, 1)]));

    let mut page_a = Ifd::new();
    page_a.set(256, Value::Short(vec![640]));
    let mut page_b = Ifd::new();
    page_b.set(256, Value::Long(vec![9]));
    page_b.set(258, Value::Short(vec![8, 8, 8])); // out of line (classic)

    let mut grandchild = Ifd::new();
    grandchild.set(33434, Value::Rational(vec![(1, 200)]));
    let mut child = Ifd::new();
    child.set(256, Value::Short(vec![16]));
    child.set_sub_ifd(tags::EXIF_IFD, vec![grandchild]);
    let mut root = Ifd::new();
    root.set(256, Value::Short(vec![640]));
    root.set_sub_ifd(tags::SUB_IFDS, vec![child]);

    vec![
        TiffFile {
            order,
            variant,
            ifds: vec![flat],
        },
        TiffFile {
            order,
            variant,
            ifds: vec![page_a, page_b],
        },
        TiffFile {
            order,
            variant,
            ifds: vec![root],
        },
    ]
}

fn orders_and_variants() -> Vec<(ByteOrder, Variant)> {
    let classic = vec![
        (ByteOrder::LittleEndian, Variant::Classic),
        (ByteOrder::BigEndian, Variant::Classic),
    ];
    #[cfg(not(feature = "bigtiff"))]
    return classic;
    #[cfg(feature = "bigtiff")]
    {
        let mut all = classic;
        all.push((ByteOrder::LittleEndian, Variant::Big));
        all.push((ByteOrder::BigEndian, Variant::Big));
        all
    }
}

/// One truth, three source shapes: for every fixture, the streaming parse must equal the slice
/// parse through `&[u8]`, through a seekable stream, and through a rebased view of the same
/// bytes embedded at a nonzero offset in a larger buffer.
#[test]
fn streaming_equals_slice_across_source_shapes() {
    for (order, variant) in orders_and_variants() {
        for file in fixture_files(order, variant) {
            let bytes = write(&file).expect("write");
            let flat = read(&bytes).expect("slice read");
            let tree = read_tree(&bytes, tags::STANDARD_POINTER_TAGS).expect("slice read_tree");
            assert_eq!(tree, file, "fixture round-trips");

            let mut slice_reader = IfdReader::open(&bytes[..]).expect("open slice");
            assert_eq!(slice_reader.read_file().expect("read_file"), flat);
            assert_eq!(
                slice_reader
                    .read_tree(tags::STANDARD_POINTER_TAGS)
                    .expect("read_tree"),
                tree
            );

            let cursor = std::io::Cursor::new(bytes.clone());
            let mut stream_reader =
                IfdReader::open(StreamSource::new(cursor)).expect("open stream");
            assert_eq!(stream_reader.read_file().expect("read_file"), flat);
            assert_eq!(
                stream_reader
                    .read_tree(tags::STANDARD_POINTER_TAGS)
                    .expect("read_tree"),
                tree
            );

            // The offset-preserving embed: the same TIFF stream at offset 41 of a larger buffer
            // (an odd offset, so nothing accidentally stays aligned).
            let mut outer = vec![0xEE; 41];
            outer.extend_from_slice(&bytes);
            outer.extend_from_slice(&[0xEE; 23]);
            let mut rebased_reader =
                IfdReader::open((&outer[..]).rebased(41)).expect("open rebased");
            assert_eq!(rebased_reader.read_file().expect("read_file"), flat);
            assert_eq!(
                rebased_reader
                    .read_tree(tags::STANDARD_POINTER_TAGS)
                    .expect("read_tree"),
                tree
            );
        }
    }
}

/// Coverage parity: the streaming accounting produces the identical report and unknown list the
/// slice accounting does, over every fixture.
#[test]
fn streaming_coverage_reports_equal_slice_reports() {
    for (order, variant) in orders_and_variants() {
        for file in fixture_files(order, variant) {
            let bytes = write(&file).expect("write");

            let mut slice_cov = Coverage::new(bytes.len() as u64);
            let mut slice_unknown = Vec::new();
            let slice_file = read_with_coverage(&bytes, &mut slice_cov, &mut slice_unknown)
                .expect("slice coverage read");

            let mut reader = IfdReader::open(&bytes[..]).expect("open");
            let mut cov = Coverage::new(bytes.len() as u64);
            let mut unknown = Vec::new();
            let stream_file = reader
                .read_file_with_coverage(&mut cov, &mut unknown)
                .expect("stream coverage read");

            assert_eq!(stream_file, slice_file);
            assert_eq!(unknown, slice_unknown);
            assert_eq!(cov.finish(), slice_cov.finish());
        }
    }
}

/// A [`ReadAt`] wrapper that counts the bytes fetched, pinning the laziness contract.
struct Counting<S> {
    inner: S,
    bytes_read: u64,
}

impl<S: ReadAt> ReadAt for Counting<S> {
    fn read_exact_at(&mut self, offset: u64, buf: &mut [u8]) -> Result<()> {
        self.bytes_read += buf.len() as u64;
        self.inner.read_exact_at(offset, buf)
    }

    fn len(&mut self) -> Result<u64> {
        self.inner.len()
    }
}

/// The laziness contract: opening, reading one directory, and decoding one value from a file
/// with megabytes of (strip-like) payload reads only the header, that directory's body, and
/// that value's span — not the payload.
#[test]
fn lazy_reads_touch_only_what_is_asked_for() {
    let mut ifd = Ifd::new();
    ifd.set(256, Value::Short(vec![640]));
    ifd.set(258, Value::Short(vec![8, 8, 8])); // 6 bytes, out of line
    ifd.set(273, Value::Long(vec![0x10_0000])); // "StripOffsets": points into the payload
    let bytes = write(&TiffFile {
        order: ByteOrder::LittleEndian,
        variant: Variant::Classic,
        ifds: vec![ifd],
    })
    .expect("write");
    // Two megabytes of payload after the directory — the part a lazy parse must never touch.
    let mut data = bytes;
    data.resize(2 * 1024 * 1024, 0xAB);

    let mut reader = IfdReader::open(Counting {
        inner: &data[..],
        bytes_read: 0,
    })
    .expect("open");
    let raw = reader
        .read_ifd(reader.first_ifd_offset())
        .expect("read_ifd");
    let out_of_line = raw.entry(258).expect("entry").clone();
    assert_eq!(
        reader.value(&out_of_line).expect("value"),
        Value::Short(vec![8, 8, 8])
    );
    let read_so_far = reader.source_mut().bytes_read;
    // Header (≤16) + count (2) + body (3 entries × 12 + 4) + the 6-byte value: 64 is generous,
    // two megabytes is the failure mode.
    assert!(read_so_far <= 64, "lazy parse read {read_so_far} bytes");
    // Decoding the whole directory re-fetches the one out-of-line value but still never touches
    // the payload: the strip *offset* is just an inline integer, and the strip bytes themselves
    // are never this crate's to read.
    let decoded = reader.decode_ifd(&raw).expect("decode");
    assert_eq!(decoded.get_u32(273), Some(0x10_0000));
    assert!(reader.source_mut().bytes_read <= read_so_far + 16);
}

/// The maker-note pattern end to end: a mini-IFD whose internal offsets are relative to the
/// note start, embedded at an arbitrary offset — parsed with `with_layout` over a rebased view,
/// out-of-line values resolving through the rebase.
#[test]
fn maker_note_mini_ifd_parses_through_a_rebased_view() {
    // The note is itself a headered TIFF stream (offsets relative to its own start), the layout
    // vendors like Olympus/Panasonic use; the enclosing "file" carries it at offset 1000.
    let mut note_ifd = Ifd::new();
    note_ifd.set(1, Value::Short(vec![7]));
    note_ifd.set(2, Value::Ascii("vendor mode".to_owned())); // out of line within the note
    let note = write(&TiffFile {
        order: ByteOrder::LittleEndian,
        variant: Variant::Classic,
        ifds: vec![note_ifd.clone()],
    })
    .expect("write note");
    let mut outer = vec![0u8; 1000];
    outer.extend_from_slice(&note);

    // A consumer that knows the layout drives the directory directly (no header re-parse)…
    let mut reader = IfdReader::with_layout(
        (&outer[..]).rebased(1000),
        ByteOrder::LittleEndian,
        Variant::Classic,
    );
    let raw = reader.read_ifd(8).expect("note directory");
    assert_eq!(reader.decode_ifd(&raw).expect("decode"), note_ifd);
    let text = raw.entry(2).expect("ascii entry").clone();
    assert_eq!(
        reader.value(&text).expect("value through rebase"),
        Value::Ascii("vendor mode".to_owned())
    );
    // …and one that lets the note's own header speak gets the same directory.
    let mut headered = IfdReader::open((&outer[..]).rebased(1000)).expect("open");
    assert_eq!(
        headered.read_file().expect("read_file").ifds,
        vec![note_ifd]
    );
}

/// The stream shape a decoder actually uses: a real file on disk, parsed through
/// `StreamSource<File>` without reading it into memory.
#[test]
fn parses_a_real_file_through_stream_source() {
    let mut grandchild = Ifd::new();
    grandchild.set(33434, Value::Rational(vec![(1, 200)]));
    let mut child = Ifd::new();
    child.set(256, Value::Short(vec![16]));
    child.set_sub_ifd(tags::EXIF_IFD, vec![grandchild]);
    let mut root = Ifd::new();
    root.set(256, Value::Short(vec![640]));
    root.set_sub_ifd(tags::SUB_IFDS, vec![child]);
    let file = TiffFile {
        order: ByteOrder::BigEndian,
        variant: Variant::Classic,
        ifds: vec![root],
    };
    let bytes = write(&file).expect("write");

    let path = std::env::temp_dir().join(format!(
        "gamut-ifd-streaming-{}-{:p}.tiff",
        std::process::id(),
        &file
    ));
    std::fs::write(&path, &bytes).expect("write temp file");
    let handle = std::fs::File::open(&path).expect("open temp file");
    let mut reader = IfdReader::open(StreamSource::new(handle)).expect("open reader");
    let parsed = reader
        .read_tree(tags::STANDARD_POINTER_TAGS)
        .expect("read_tree");
    std::fs::remove_file(&path).expect("remove temp file");
    assert_eq!(parsed, file);
}
