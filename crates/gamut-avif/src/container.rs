//! The total, byte-accounting representation of an AVIF file ([`AvifContainer`]).
//!
//! Mirroring the guarantee `gamut-heic` established for HEIF (and issue #250 asks of AVIF), it is
//! *structurally impossible to ignore any bits* in the container: every byte of the input is
//! mapped to exactly one [`Segment`], the segments are contiguous and non-overlapping, and
//! together they cover `0..data.len()` exactly. Unknown top-level boxes are surfaced verbatim
//! (never dropped), an appended foreign stream (a second top-level `ftyp`, e.g. a phone
//! motion-photo MP4) is retained as an opaque byte range, and any trailing non-box bytes become an
//! explicit [`SegmentKind::Trailer`].
//!
//! The top-level walk uses [`gamut_isobmff::BoxReader`] and stops on exactly the same rules as
//! [`gamut_isobmff::read`] — the first `ftyp` wins, a second top-level `ftyp` begins the appended
//! stream, and a malformed trailing box is tolerated (as a trailer) only once both `ftyp` and
//! `meta` have been seen — so the byte-accounting walk and the semantic parse never disagree.

use core::ops::Range;

use gamut_core::Result;
use gamut_isobmff::{BoxReader, read};

use crate::image::AvifImage;

/// An AVIF file decomposed into a byte-exact list of [`Segment`]s plus the semantic [`AvifImage`].
///
/// # The every-byte invariant
///
/// [`segments`](Self::segments) is contiguous, non-overlapping, and covers `0..data.len()`
/// exactly: `segments[0].range.start == 0`, each `segments[i].range.end ==
/// segments[i+1].range.start`, and the last `range.end == data.len()`. This is the "structurally
/// impossible to ignore any bits" guarantee issue #250 inherits from the HEIF surface — it holds
/// by construction and is pinned by the crate's tests. Every primary-stream top-level box
/// (including unrecognised ones such as a Google `mpvd`) is a [`SegmentKind::Box`]; an appended
/// foreign stream is a single [`SegmentKind::AppendedStream`]; any trailing non-box bytes are a
/// single [`SegmentKind::Trailer`].
///
/// The [`image`](Self::image) field is the role-typed semantic view of the *primary* still-image
/// stream. Boxes inside `meta` that [`gamut_isobmff::read`] does not consume (e.g. a `dinf`/`dref`
/// pair or a `uuid` box) are surfaced in [`unknown_meta_boxes`](Self::unknown_meta_boxes) so that,
/// at the meta level too, nothing is silently dropped.
#[derive(Debug)]
pub struct AvifContainer<'a> {
    data: &'a [u8],
    segments: Vec<Segment<'a>>,
    image: AvifImage,
    unknown_meta_boxes: Vec<UnknownBox<'a>>,
}

/// One contiguous run of the input file, tagged by what it holds ([`SegmentKind`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment<'a> {
    /// The half-open byte range this segment occupies within the input (`start..end`).
    pub range: Range<usize>,
    /// What the bytes in [`range`](Self::range) are.
    pub kind: SegmentKind<'a>,
}

/// What a [`Segment`] holds.
///
/// Non-exhaustive: a future revision may add a variant (e.g. a typed motion-photo marker) without
/// a breaking change — match with a wildcard arm.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum SegmentKind<'a> {
    /// A top-level box of the primary stream: its four-character type and its body (the bytes
    /// after the 8-byte header). Recognised (`ftyp`/`meta`/`mdat`/`free`/`idat`…) and unrecognised
    /// (e.g. a Google Motion Photo `mpvd`) boxes alike are surfaced here — the container never
    /// drops one.
    Box {
        /// The box's four-character type.
        ty: [u8; 4],
        /// The box body (everything after the 8-byte size+type header).
        body: &'a [u8],
    },
    /// An appended foreign stream: everything from a *second* top-level `ftyp` to end of file,
    /// kept opaque. Real-world phones append a whole second file here (a motion-photo MP4 with its
    /// own `moov`, optionally followed by a proprietary trailer); its vendor semantics stay
    /// downstream.
    AppendedStream(&'a [u8]),
    /// Trailing non-box bytes: from the first malformed/truncated top-level box header to end of
    /// file. Only permitted after both `ftyp` and `meta` have been parsed (before that a malformed
    /// box is a parse error) — matching [`gamut_isobmff::read`]'s tolerance rule exactly.
    Trailer(&'a [u8]),
}

/// A box found inside `meta` (or `meta`'s `iprp`) that the semantic [`gamut_isobmff::read`] does
/// not consume, surfaced verbatim so meta-level accounting is complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownBox<'a> {
    /// Where the box was found — a direct child of `meta` or of `meta`'s `iprp`.
    pub location: UnknownBoxLocation,
    /// The box's four-character type (e.g. `*b"uuid"`, `*b"dinf"`).
    pub ty: [u8; 4],
    /// The box body (everything after the 8-byte size+type header).
    pub body: &'a [u8],
}

/// The container level at which an [`UnknownBox`] was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnknownBoxLocation {
    /// A direct child of the `meta` box (siblings of `hdlr`/`pitm`/`iloc`/`iinf`/`iref`/`iprp`/
    /// `idat`/`grpl`).
    Meta,
    /// A direct child of `meta`'s `iprp` box (siblings of `ipco`/`ipma`).
    Iprp,
}

impl<'a> AvifContainer<'a> {
    /// Parses `data` into the total byte-accounting representation and the role-typed semantic
    /// view.
    ///
    /// The semantic layer is [`gamut_isobmff::read`] wrapped in an [`AvifImage`], which
    /// additionally validates the primary item at parse time (the `pitm` id must name an existing,
    /// non-hidden item — ISO/IEC 23008-12). The byte-accounting layer walks the top-level boxes
    /// into [`segments`](Self::segments) covering every byte, and shadow-walks `meta` for boxes
    /// `read` does not consume ([`unknown_meta_boxes`](Self::unknown_meta_boxes)).
    ///
    /// # Errors
    ///
    /// Propagates every [`gamut_isobmff::read`] error (a missing/truncated required box, an
    /// out-of-scope feature such as an `avis` image sequence, …) and returns
    /// [`Error::InvalidInput`](gamut_core::Error::InvalidInput) if the primary item is missing or
    /// hidden. A malformed top-level box *before* both `ftyp` and `meta` are seen is a parse
    /// error; after that it is retained as a [`SegmentKind::Trailer`].
    pub fn parse(data: &'a [u8]) -> Result<Self> {
        // Semantic parse first: it enforces the stricter rules (rejecting out-of-scope
        // `moov`/`trak`, missing required boxes, …). Because it succeeds only once `ftyp`+`meta`
        // are seen and uses the identical `BoxReader` walk, the byte-accounting walk below cannot
        // then disagree.
        let image = AvifImage::new(read(data)?)?;
        let (segments, meta_body) = walk_segments(data)?;
        let unknown_meta_boxes = match meta_body {
            Some(body) => shadow_walk_meta(body)?,
            // Unreachable once `read` has succeeded (it requires `meta`); an empty list is the
            // honest answer if it ever were.
            None => Vec::new(),
        };
        Ok(Self {
            data,
            segments,
            image,
            unknown_meta_boxes,
        })
    }

    /// The whole input buffer this container was parsed from.
    #[must_use]
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    /// The byte-exact segment list. Contiguous, non-overlapping, and covering `0..data.len()`
    /// exactly (see the [type invariant](Self#the-every-byte-invariant)).
    #[must_use]
    pub fn segments(&self) -> &[Segment<'a>] {
        &self.segments
    }

    /// Iterates the primary-stream top-level boxes as `(type, body)` pairs, in file order — the
    /// [`SegmentKind::Box`] segments only.
    pub fn boxes(&self) -> impl Iterator<Item = ([u8; 4], &'a [u8])> + '_ {
        self.segments.iter().filter_map(|s| match &s.kind {
            SegmentKind::Box { ty, body } => Some((*ty, *body)),
            _ => None,
        })
    }

    /// The appended foreign stream (from a second top-level `ftyp` to EOF), if the file has one.
    #[must_use]
    pub fn appended_stream(&self) -> Option<&'a [u8]> {
        self.segments.iter().find_map(|s| match s.kind {
            SegmentKind::AppendedStream(bytes) => Some(bytes),
            _ => None,
        })
    }

    /// The trailing non-box bytes (a malformed/truncated box header to EOF), if the file has any.
    #[must_use]
    pub fn trailer(&self) -> Option<&'a [u8]> {
        self.segments.iter().find_map(|s| match s.kind {
            SegmentKind::Trailer(bytes) => Some(bytes),
            _ => None,
        })
    }

    /// The role-typed semantic view of the primary still-image stream.
    #[must_use]
    pub fn image(&self) -> &AvifImage {
        &self.image
    }

    /// The boxes inside `meta`/`iprp` that the semantic parse did not consume, surfaced verbatim.
    #[must_use]
    pub fn unknown_meta_boxes(&self) -> &[UnknownBox<'a>] {
        &self.unknown_meta_boxes
    }

    /// Decodes an item to a raw planar [`DecodedFrame`](crate::DecodedFrame) via `decoder`.
    /// Convenience forwarding to
    /// [`AvifImage::decode_item_planar`](crate::AvifImage::decode_item_planar).
    ///
    /// # Errors
    ///
    /// As [`AvifImage::decode_item_planar`](crate::AvifImage::decode_item_planar).
    pub fn decode_item_planar(
        &self,
        id: u32,
        decoder: &mut dyn crate::Av1StillDecoder,
    ) -> Result<crate::DecodedFrame> {
        self.image.decode_item_planar(id, decoder)
    }
}

/// Walks the top-level boxes into a contiguous, gap-free segment list covering `0..data.len()`,
/// also returning the `meta` box body (the last one, matching [`gamut_isobmff::read`]) for the
/// meta-level shadow walk.
///
/// Stops — and closes out the remaining bytes as one segment — at a second top-level `ftyp`
/// (appended stream) or at a malformed trailing box once `ftyp`+`meta` are seen (trailer).
fn walk_segments(data: &[u8]) -> Result<(Vec<Segment<'_>>, Option<&[u8]>)> {
    let mut segments = Vec::new();
    let mut reader = BoxReader::new(data);
    let mut seen_ftyp = false;
    let mut seen_meta = false;
    let mut meta_body = None;
    loop {
        let box_start = reader.position();
        match reader.next_box() {
            Ok(Some(b)) => {
                // A second top-level `ftyp` begins the appended foreign stream: everything from
                // this header to EOF is one opaque segment. The first `ftyp` wins.
                if &b.ty == b"ftyp" && seen_ftyp {
                    segments.push(Segment {
                        range: b.offset..data.len(),
                        kind: SegmentKind::AppendedStream(&data[b.offset..]),
                    });
                    break;
                }
                let end = b.offset + 8 + b.body.len();
                segments.push(Segment {
                    range: b.offset..end,
                    kind: SegmentKind::Box {
                        ty: b.ty,
                        body: b.body,
                    },
                });
                match &b.ty {
                    b"ftyp" => seen_ftyp = true,
                    b"meta" => {
                        seen_meta = true;
                        meta_body = Some(b.body);
                    }
                    _ => {}
                }
            }
            // Clean end of the box list: the last box tiled exactly to EOF, so the segments
            // already cover every byte.
            Ok(None) => break,
            Err(e) => {
                // A malformed trailing box after both required boxes are seen is retained as a
                // trailer (matching `read`). Before that, the file itself is malformed: propagate.
                if seen_ftyp && seen_meta {
                    segments.push(Segment {
                        range: box_start..data.len(),
                        kind: SegmentKind::Trailer(&data[box_start..]),
                    });
                    break;
                }
                return Err(e);
            }
        }
    }
    Ok((segments, meta_body))
}

/// Shadow-walks a `meta` box body for boxes the semantic parse does not consume.
///
/// `meta` is a `FullBox`, so the first 4 bytes are its version+flags; its children then follow.
/// The consumed direct children are `hdlr`/`pitm`/`iloc`/`iinf`/`iref`/`iprp`/`idat`/`grpl`; every
/// other direct child is captured with [`UnknownBoxLocation::Meta`]. Descending into `iprp` (which
/// is a plain box, not a `FullBox`), the consumed children are `ipco`/`ipma`; every other is
/// captured with [`UnknownBoxLocation::Iprp`]. `ipco`/`iinf`/`iref` children are already fully
/// modelled by the semantic layer (as properties/items/references) and are never double-reported.
fn shadow_walk_meta(meta_body: &[u8]) -> Result<Vec<UnknownBox<'_>>> {
    let mut unknown = Vec::new();
    // Skip the FullBox version+flags. `read` already validated `meta`, so the header is present;
    // an empty child region is the honest fallback if not.
    let children = meta_body.get(4..).unwrap_or(&[]);
    let mut reader = BoxReader::new(children);
    while let Some(b) = reader.next_box()? {
        match &b.ty {
            b"iprp" => {
                let mut iprp = BoxReader::new(b.body);
                while let Some(c) = iprp.next_box()? {
                    if !matches!(&c.ty, b"ipco" | b"ipma") {
                        unknown.push(UnknownBox {
                            location: UnknownBoxLocation::Iprp,
                            ty: c.ty,
                            body: c.body,
                        });
                    }
                }
            }
            b"hdlr" | b"pitm" | b"iloc" | b"iinf" | b"iref" | b"idat" | b"grpl" => {}
            _ => unknown.push(UnknownBox {
                location: UnknownBoxLocation::Meta,
                ty: b.ty,
                body: b.body,
            }),
        }
    }
    Ok(unknown)
}
