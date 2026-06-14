//! Parses a single-still-image ISOBMFF file into an [`IsoBmffImage`].
//!
//! The structure is offset-driven (a parser-exploit surface), so every read is bounds-checked via
//! [`BoxReader`], counts are capped against the remaining bytes before looping, and out-of-scope
//! features (tracks, `iloc` v1/v2, multi-extent, ICC `colr`) are rejected or preserved verbatim
//! rather than mis-parsed. Only what [`crate::write`] can produce is round-tripped; foreign files
//! (sequences, `free` boxes, multi-extent items) are out of scope. See `references/isobmff`.

use gamut_core::{Error, Result};

use crate::boxes::BoxReader;
use crate::model::{ColourInformation, IsoBmffImage, Item, NclxColr, Property, PropertyKind};

/// Per-item property associations parsed from `ipma`: each entry is `(item_id, associations)` where
/// an association is `(property_index, essential)`.
type ItemAssociations = Vec<(u16, Vec<(u16, bool)>)>;

/// Parses `data` into an [`IsoBmffImage`].
///
/// # Errors
///
/// Returns [`Error::InvalidInput`] if a box is truncated or overruns, a required box is missing, an
/// `iloc` extent points outside the file, an `ipma` property index is out of range, or an `infe`
/// name is not UTF-8; [`Error::Unsupported`] for structurally valid but out-of-scope features
/// (image sequences/tracks, `iloc` v1/v2, multiple extents, 16-bit `ipma` indices, a non-`pict`
/// handler).
pub fn read(data: &[u8]) -> Result<IsoBmffImage> {
    let mut top = BoxReader::new(data);
    let mut ftyp = None;
    let mut meta_body = None;
    let mut saw_mdat = false;
    while let Some(b) = top.next_box()? {
        match &b.ty {
            b"ftyp" => ftyp = Some(parse_ftyp(b.body)?),
            b"meta" => meta_body = Some(b.body),
            b"mdat" => saw_mdat = true,
            b"moov" | b"trak" => {
                return Err(Error::Unsupported(
                    "ISOBMFF: image sequences (tracks) not supported",
                ));
            }
            _ => {} // tolerate benign unknown top-level boxes (e.g. free/skip)
        }
    }

    let (major_brand, minor_version, compatible_brands) =
        ftyp.ok_or(Error::InvalidInput("ISOBMFF: missing ftyp"))?;
    let meta_body = meta_body.ok_or(Error::InvalidInput("ISOBMFF: missing meta"))?;
    if !saw_mdat {
        return Err(Error::InvalidInput("ISOBMFF: missing mdat"));
    }
    let meta = parse_meta(meta_body)?;

    // Assemble items, resolving each item's payload from its absolute `iloc` extent into `data`.
    let mut items = Vec::with_capacity(meta.infe.len());
    for infe in &meta.infe {
        let loc = meta
            .iloc
            .iter()
            .find(|e| e.id == infe.id)
            .ok_or(Error::InvalidInput("ISOBMFF: iloc missing item"))?;
        let start = loc.offset as usize;
        let end = start
            .checked_add(loc.length as usize)
            .ok_or(Error::InvalidInput("ISOBMFF: iloc extent overflow"))?;
        let payload = data
            .get(start..end)
            .ok_or(Error::InvalidInput("ISOBMFF: iloc extent out of bounds"))?
            .to_vec();

        let assoc = meta
            .ipma
            .iter()
            .find(|(id, _)| *id == infe.id)
            .map(|(_, row)| row)
            .ok_or(Error::InvalidInput("ISOBMFF: ipma missing item"))?;
        let mut properties = Vec::with_capacity(assoc.len());
        for &(index, essential) in assoc {
            let i = usize::from(index);
            if index == 0 || i > meta.ipco.len() {
                return Err(Error::InvalidInput(
                    "ISOBMFF: ipma property index out of range",
                ));
            }
            properties.push(Property {
                essential,
                kind: meta.ipco[i - 1].clone(),
            });
        }
        items.push(Item {
            id: infe.id,
            item_type: infe.item_type,
            name: infe.name.clone(),
            properties,
            payload,
        });
    }

    Ok(IsoBmffImage {
        major_brand,
        minor_version,
        compatible_brands,
        primary_item_id: meta.primary_item_id,
        items,
    })
}

/// `ftyp`: major brand, minor version, and 4-byte compatible brands.
fn parse_ftyp(body: &[u8]) -> Result<([u8; 4], u32, Vec<[u8; 4]>)> {
    let mut r = BoxReader::new(body);
    let major = r.fourcc()?;
    let minor = r.u32()?;
    let mut compatible = Vec::with_capacity(r.remaining() / 4);
    while r.remaining() >= 4 {
        compatible.push(r.fourcc()?);
    }
    if r.remaining() != 0 {
        return Err(Error::InvalidInput("ISOBMFF: ftyp has trailing bytes"));
    }
    Ok((major, minor, compatible))
}

/// The structural pieces parsed out of `meta`, before payloads are resolved.
struct Meta {
    primary_item_id: u16,
    iloc: Vec<IlocEntry>,
    infe: Vec<InfeEntry>,
    ipco: Vec<PropertyKind>,
    ipma: ItemAssociations,
}

/// One `iloc` item: its id and single resolved extent (absolute file offset + length).
struct IlocEntry {
    id: u16,
    offset: u32,
    length: u32,
}

/// One `infe` entry: item id, type, and name.
struct InfeEntry {
    id: u16,
    item_type: [u8; 4],
    name: String,
}

/// Parses the `meta` (FullBox) body and its child boxes.
fn parse_meta(body: &[u8]) -> Result<Meta> {
    let mut r = BoxReader::new(body);
    full_box_header(&mut r)?; // meta is a FullBox

    let mut primary_item_id = None;
    let mut iloc = None;
    let mut infe = None;
    let mut iprp = None;
    while let Some(b) = r.next_box()? {
        match &b.ty {
            b"hdlr" => parse_hdlr(b.body)?,
            b"pitm" => primary_item_id = Some(parse_pitm(b.body)?),
            b"iloc" => iloc = Some(parse_iloc(b.body)?),
            b"iinf" => infe = Some(parse_iinf(b.body)?),
            b"iprp" => iprp = Some(parse_iprp(b.body)?),
            _ => {} // tolerate unknown meta children
        }
    }

    let (ipco, ipma) = iprp.ok_or(Error::InvalidInput("ISOBMFF: meta missing iprp"))?;
    Ok(Meta {
        primary_item_id: primary_item_id
            .ok_or(Error::InvalidInput("ISOBMFF: meta missing pitm"))?,
        iloc: iloc.ok_or(Error::InvalidInput("ISOBMFF: meta missing iloc"))?,
        infe: infe.ok_or(Error::InvalidInput("ISOBMFF: meta missing iinf"))?,
        ipco,
        ipma,
    })
}

/// Reads a `FullBox` header, returning the version and skipping the 3 flags bytes. No box in the
/// still-image profile reads flags except `ipma` (which reads its single relevant bit inline), so
/// the flags are not decoded here.
fn full_box_header(r: &mut BoxReader) -> Result<u8> {
    let version = r.u8()?;
    r.take(3)?; // flags (unused in this profile)
    Ok(version)
}

/// `hdlr`: require `handler_type == "pict"` (HEIF image).
fn parse_hdlr(body: &[u8]) -> Result<()> {
    let mut r = BoxReader::new(body);
    full_box_header(&mut r)?;
    let _pre_defined = r.u32()?;
    let handler = r.fourcc()?;
    if &handler != b"pict" {
        return Err(Error::Unsupported("ISOBMFF: non-picture handler"));
    }
    Ok(())
}

/// `pitm` v0: the primary item id.
fn parse_pitm(body: &[u8]) -> Result<u16> {
    let mut r = BoxReader::new(body);
    if full_box_header(&mut r)? != 0 {
        return Err(Error::Unsupported(
            "ISOBMFF: pitm version (only v0 supported)",
        ));
    }
    r.u16()
}

/// `iloc` v0: one extent per item, `construction_method` 0.
fn parse_iloc(body: &[u8]) -> Result<Vec<IlocEntry>> {
    let mut r = BoxReader::new(body);
    if full_box_header(&mut r)? != 0 {
        return Err(Error::Unsupported(
            "ISOBMFF: iloc version (only v0 supported)",
        ));
    }
    let sizes = r.u8()?;
    if sizes != 0x44 {
        return Err(Error::Unsupported(
            "ISOBMFF: iloc offset_size/length_size != 4",
        ));
    }
    let base = r.u8()?;
    if base & 0xf0 != 0 {
        return Err(Error::Unsupported("ISOBMFF: iloc base_offset_size != 0"));
    }
    // `item_count` is untrusted; do not pre-allocate from it — the bounded reads below fail on
    // truncation, so a malformed count errors after a bounded number of iterations.
    let item_count = r.u16()?;
    let mut entries = Vec::new();
    for _ in 0..item_count {
        let id = r.u16()?;
        let _data_reference_index = r.u16()?;
        let extent_count = r.u16()?;
        if extent_count != 1 {
            return Err(Error::Unsupported("ISOBMFF: iloc multiple extents"));
        }
        let offset = r.u32()?;
        let length = r.u32()?;
        entries.push(IlocEntry { id, offset, length });
    }
    Ok(entries)
}

/// `iinf` v0 + `infe` v2 children.
fn parse_iinf(body: &[u8]) -> Result<Vec<InfeEntry>> {
    let mut r = BoxReader::new(body);
    if full_box_header(&mut r)? != 0 {
        return Err(Error::Unsupported(
            "ISOBMFF: iinf version (only v0 supported)",
        ));
    }
    let entry_count = r.u16()?;
    let mut entries = Vec::new();
    for _ in 0..entry_count {
        let b = r
            .next_box()?
            .ok_or(Error::InvalidInput("ISOBMFF: iinf truncated"))?;
        if &b.ty != b"infe" {
            return Err(Error::InvalidInput("ISOBMFF: iinf child is not infe"));
        }
        entries.push(parse_infe(b.body)?);
    }
    Ok(entries)
}

/// `infe` v2: item id, type, name.
fn parse_infe(body: &[u8]) -> Result<InfeEntry> {
    let mut r = BoxReader::new(body);
    if full_box_header(&mut r)? != 2 {
        return Err(Error::Unsupported(
            "ISOBMFF: infe version (only v2 supported)",
        ));
    }
    let id = r.u16()?;
    let _item_protection_index = r.u16()?;
    let item_type = r.fourcc()?;
    let name = read_c_string(&mut r)?;
    Ok(InfeEntry {
        id,
        item_type,
        name,
    })
}

/// Reads a NUL-terminated UTF-8 string (without the terminator), tolerating a missing terminator at
/// end of box.
fn read_c_string(r: &mut BoxReader) -> Result<String> {
    let mut bytes = Vec::new();
    while r.remaining() != 0 {
        let b = r.u8()?;
        if b == 0 {
            break;
        }
        bytes.push(b);
    }
    String::from_utf8(bytes).map_err(|_| Error::InvalidInput("ISOBMFF: infe name not UTF-8"))
}

/// `iprp`: the `ipco` property list (1-based) and the `ipma` per-item associations.
fn parse_iprp(body: &[u8]) -> Result<(Vec<PropertyKind>, ItemAssociations)> {
    let mut r = BoxReader::new(body);
    let mut ipco = None;
    let mut ipma = None;
    while let Some(b) = r.next_box()? {
        match &b.ty {
            b"ipco" => ipco = Some(parse_ipco(b.body)?),
            b"ipma" => ipma = Some(parse_ipma(b.body)?),
            _ => {}
        }
    }
    let ipco = ipco.ok_or(Error::InvalidInput("ISOBMFF: iprp missing ipco"))?;
    let ipma = ipma.ok_or(Error::InvalidInput("ISOBMFF: iprp missing ipma"))?;
    Ok((ipco, ipma))
}

/// `ipco`: the ordered property container; each child box becomes one [`PropertyKind`].
fn parse_ipco(body: &[u8]) -> Result<Vec<PropertyKind>> {
    let mut r = BoxReader::new(body);
    let mut props = Vec::new();
    while let Some(b) = r.next_box()? {
        props.push(parse_property(b.ty, b.body)?);
    }
    Ok(props)
}

/// Maps one `ipco` child box to a [`PropertyKind`]. Unrecognised boxes are preserved verbatim.
fn parse_property(ty: [u8; 4], body: &[u8]) -> Result<PropertyKind> {
    match &ty {
        b"ispe" => {
            let mut r = BoxReader::new(body);
            full_box_header(&mut r)?;
            let width = r.u32()?;
            let height = r.u32()?;
            Ok(PropertyKind::ImageSpatialExtents { width, height })
        }
        b"pixi" => {
            let mut r = BoxReader::new(body);
            full_box_header(&mut r)?;
            let count = r.u8()?;
            let mut bits_per_channel = Vec::new();
            for _ in 0..count {
                bits_per_channel.push(r.u8()?);
            }
            Ok(PropertyKind::PixelInformation { bits_per_channel })
        }
        b"colr" => {
            let mut r = BoxReader::new(body);
            let colour_type = r.fourcc()?;
            if &colour_type == b"nclx" {
                let colour_primaries = r.u16()?;
                let transfer_characteristics = r.u16()?;
                let matrix_coefficients = r.u16()?;
                let full_range = (r.u8()? >> 7) & 1 == 1;
                Ok(PropertyKind::Colour(ColourInformation::Nclx(NclxColr {
                    colour_primaries,
                    transfer_characteristics,
                    matrix_coefficients,
                    full_range,
                })))
            } else {
                // ICC (rICC/prof) and other colour types: preserve verbatim.
                Ok(PropertyKind::Other {
                    kind: ty,
                    data: body.to_vec(),
                })
            }
        }
        b"irot" => {
            let mut r = BoxReader::new(body);
            Ok(PropertyKind::Rotation(r.u8()? & 0x03))
        }
        b"imir" => {
            let mut r = BoxReader::new(body);
            Ok(PropertyKind::Mirror(r.u8()? & 0x01))
        }
        b"av1C" => Ok(PropertyKind::CodecConfiguration {
            kind: ty,
            data: body.to_vec(),
        }),
        _ => Ok(PropertyKind::Other {
            kind: ty,
            data: body.to_vec(),
        }),
    }
}

/// `ipma` v0 (single-byte associations): each item id → its `(property_index, essential)` list.
fn parse_ipma(body: &[u8]) -> Result<ItemAssociations> {
    let mut r = BoxReader::new(body);
    // `ipma` is the one box whose flags matter: `flags & 1` selects 16-bit property indices, which
    // this profile does not support. Read version + the flags byte directly (the upper two flag
    // bytes carry no meaning here).
    let version = r.u8()?;
    let _flags_hi = r.take(2)?;
    let flags_lo = r.u8()?;
    if version != 0 {
        return Err(Error::Unsupported(
            "ISOBMFF: ipma version (only v0 supported)",
        ));
    }
    if flags_lo & 1 == 1 {
        return Err(Error::Unsupported(
            "ISOBMFF: ipma 16-bit property indices (flags & 1)",
        ));
    }
    // `entry_count`/`association_count` are untrusted; do not pre-allocate from them — the bounded
    // reads below fail on truncation after a bounded number of iterations.
    let entry_count = r.u32()?;
    let mut out = Vec::new();
    for _ in 0..entry_count {
        let item_id = r.u16()?;
        let assoc_count = r.u8()?;
        let mut row = Vec::new();
        for _ in 0..assoc_count {
            let byte = r.u8()?;
            row.push((u16::from(byte & 0x7f), (byte & 0x80) != 0));
        }
        out.push((item_id, row));
    }
    Ok(out)
}
