//! The container variant, the IFD entries, and the directory that holds them.

use crate::value::Value;

/// Which variant of the TIFF container a stream uses, distinguished by the header magic number.
///
/// The two variants share an identical tag/IFD model; they differ only in the width of the
/// structural fields — BigTIFF widens every file offset (and the IFD entry count and per-field
/// value count) from 32 to 64 bits so a file may exceed 4 GiB (`references/tiff/bigtiff.html`).
/// The [`Variant::Big`] arm — and every 64-bit width it selects — exists only when the `bigtiff`
/// feature is enabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Variant {
    /// Classic TIFF 6.0: magic `42`, an 8-byte header, 12-byte IFD entries, 32-bit offsets.
    Classic,
    /// BigTIFF: magic `43`, a 16-byte header, 20-byte IFD entries, 64-bit offsets.
    #[cfg(feature = "bigtiff")]
    Big,
}

impl Variant {
    /// The header magic number (`42` for classic, `43` for BigTIFF).
    #[must_use]
    pub fn magic(self) -> u16 {
        match self {
            Variant::Classic => 42,
            #[cfg(feature = "bigtiff")]
            Variant::Big => 43,
        }
    }

    /// The size of the image file header in bytes (8 classic, 16 BigTIFF).
    #[must_use]
    pub fn header_size(self) -> usize {
        match self {
            Variant::Classic => 8,
            #[cfg(feature = "bigtiff")]
            Variant::Big => 16,
        }
    }

    /// The on-disk size of one IFD entry in bytes (12 classic, 20 BigTIFF).
    #[must_use]
    pub fn entry_size(self) -> usize {
        match self {
            Variant::Classic => 12,
            #[cfg(feature = "bigtiff")]
            Variant::Big => 20,
        }
    }

    /// The size of an IFD's leading entry-count field in bytes (2 classic, 8 BigTIFF).
    #[must_use]
    pub fn count_size(self) -> usize {
        match self {
            Variant::Classic => 2,
            #[cfg(feature = "bigtiff")]
            Variant::Big => 8,
        }
    }

    /// The size of a file offset (first-IFD pointer, next-IFD pointer, value offset) in bytes
    /// (4 classic, 8 BigTIFF).
    #[must_use]
    pub fn offset_size(self) -> usize {
        match self {
            Variant::Classic => 4,
            #[cfg(feature = "bigtiff")]
            Variant::Big => 8,
        }
    }

    /// The largest value, in bytes, that is stored inline in an IFD entry rather than out of line
    /// (4 classic, 8 BigTIFF — equal to [`Self::offset_size`]).
    #[must_use]
    pub fn inline_threshold(self) -> usize {
        self.offset_size()
    }
}

/// One field (entry) of an Image File Directory: a tag and its value.
///
/// On disk this is a 12-byte (classic) or 20-byte (BigTIFF) record — tag, field type, value count,
/// and a value-or-offset word — but once decoded only the tag and the resolved [`Value`] matter;
/// the field type and count are recoverable from the value.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// The 16-bit tag identifying the field (e.g. `256` for `ImageWidth`).
    pub tag: u16,
    /// The field's value.
    pub value: Value,
}

/// A sub-IFD pointer: a tag whose value is the file offset (or array of offsets) of one or more
/// nested IFDs that are **not** part of the top-level next-IFD chain.
///
/// This is how TIFF/EP and DNG attach child directories. `SubIFDs` (330) points at the
/// full-resolution raw image IFD(s); `ExifIFD` (34665) and `GPSInfo` (34853) point at the EXIF and
/// GPS sub-directories. [`write`](crate::write) lays the children out and synthesises the pointer
/// field, patching it with their absolute offset(s). The generic [`read`](crate::read) does **not**
/// follow these — it cannot know which arbitrary `LONG` tags are offsets — so a decoder reads the
/// offset value(s) and re-parses each child with [`read_ifd_at`](crate::read_ifd_at).
#[derive(Debug, Clone, PartialEq)]
pub struct SubIfd {
    /// The pointer tag (e.g. `330` for `SubIFDs`).
    pub tag: u16,
    /// The child IFD(s) the tag points at — exactly one for `ExifIFD`/`GPSInfo`, one or more for
    /// `SubIFDs`.
    pub ifds: Vec<Ifd>,
}

/// A parsed Image File Directory — one node in the IFD chain (a TIFF page, or an EXIF/GPS/Interop
/// sub-directory).
///
/// Fields are kept sorted in ascending tag order, as required on disk (TIFF 6.0 §2); the accessors
/// preserve that invariant. A directory may also carry [`sub_ifds`](Self::sub_ifds) — child
/// directories referenced by a pointer tag rather than the next-IFD chain (e.g. a DNG's raw
/// sub-IFD or its EXIF sub-IFD); the writer lays those out and synthesises their pointer fields.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ifd {
    fields: Vec<Field>,
    sub_ifds: Vec<SubIfd>,
}

impl Ifd {
    /// Creates an empty directory.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the directory's fields, sorted by ascending tag.
    #[must_use]
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    /// Returns the value of `tag`, or `None` if absent.
    #[must_use]
    pub fn get(&self, tag: u16) -> Option<&Value> {
        self.fields
            .binary_search_by_key(&tag, |f| f.tag)
            .ok()
            .map(|i| &self.fields[i].value)
    }

    /// Returns `tag` coerced to a single `u32` (accepting `BYTE`/`SHORT`/`LONG`).
    #[must_use]
    pub fn get_u32(&self, tag: u16) -> Option<u32> {
        self.get(tag).and_then(Value::as_u32)
    }

    /// Returns `tag` coerced to a `Vec<u32>` (accepting `BYTE`/`SHORT`/`LONG`).
    #[must_use]
    pub fn get_u32_vec(&self, tag: u16) -> Option<Vec<u32>> {
        self.get(tag).and_then(Value::as_u32_vec)
    }

    /// Inserts or replaces the value of `tag`, keeping the fields sorted.
    pub fn set(&mut self, tag: u16, value: Value) {
        match self.fields.binary_search_by_key(&tag, |f| f.tag) {
            Ok(i) => self.fields[i].value = value,
            Err(i) => self.fields.insert(i, Field { tag, value }),
        }
    }

    /// Returns the directory's sub-IFD pointers (see [`SubIfd`]).
    #[must_use]
    pub fn sub_ifds(&self) -> &[SubIfd] {
        &self.sub_ifds
    }

    /// Attaches `ifds` as the child directories of pointer `tag`, replacing any existing group for
    /// that tag.
    ///
    /// The pointer field (a `LONG`/`LONG8` array of the children's offsets) is synthesised by the
    /// writer, so `tag` must **not** also be [`set`](Self::set) as a regular field.
    pub fn set_sub_ifd(&mut self, tag: u16, ifds: Vec<Ifd>) {
        match self.sub_ifds.iter_mut().find(|s| s.tag == tag) {
            Some(s) => s.ifds = ifds,
            None => self.sub_ifds.push(SubIfd { tag, ifds }),
        }
    }

    /// Appends a single child directory to pointer `tag`, creating the group if it does not exist.
    pub fn add_sub_ifd(&mut self, tag: u16, ifd: Ifd) {
        match self.sub_ifds.iter_mut().find(|s| s.tag == tag) {
            Some(s) => s.ifds.push(ifd),
            None => self.sub_ifds.push(SubIfd {
                tag,
                ifds: vec![ifd],
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_layout_constants() {
        assert_eq!(Variant::Classic.magic(), 42);
        assert_eq!(Variant::Classic.header_size(), 8);
        assert_eq!(Variant::Classic.entry_size(), 12);
        assert_eq!(Variant::Classic.count_size(), 2);
        assert_eq!(Variant::Classic.offset_size(), 4);
        // The inline threshold equals the offset size: values up to that many bytes pack inline.
        assert_eq!(Variant::Classic.inline_threshold(), 4);
    }

    #[cfg(feature = "bigtiff")]
    #[test]
    fn bigtiff_variant_layout_constants() {
        assert_eq!(Variant::Big.magic(), 43);
        assert_eq!(Variant::Big.header_size(), 16);
        assert_eq!(Variant::Big.entry_size(), 20);
        assert_eq!(Variant::Big.count_size(), 8);
        assert_eq!(Variant::Big.offset_size(), 8);
        assert_eq!(Variant::Big.inline_threshold(), 8);
    }

    #[test]
    fn ifd_keeps_fields_sorted_and_replaces() {
        // Tag numbers are used literally here: tag semantics live in the consuming codec
        // (e.g. gamut-tiff's `tags` module), not in this structural core. 256/257/259 are
        // ImageWidth/ImageLength/Compression.
        let mut ifd = Ifd::new();
        ifd.set(259, Value::Short(vec![1]));
        ifd.set(256, Value::Short(vec![4]));
        ifd.set(257, Value::Short(vec![3]));
        let order: Vec<u16> = ifd.fields().iter().map(|f| f.tag).collect();
        assert_eq!(order, vec![256, 257, 259]);
        ifd.set(256, Value::Short(vec![8]));
        assert_eq!(ifd.get_u32(256), Some(8));
        assert_eq!(ifd.fields().len(), 3);
    }
}
