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

/// A parsed Image File Directory — one node in the IFD chain (a TIFF page, or an EXIF/GPS/Interop
/// sub-directory).
///
/// Fields are kept sorted in ascending tag order, as required on disk (TIFF 6.0 §2); the accessors
/// preserve that invariant.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Ifd {
    fields: Vec<Field>,
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

    #[test]
    fn get_u32_vec_reads_arrays_and_misses() {
        let mut ifd = Ifd::new();
        ifd.set(258, Value::Short(vec![8, 8, 8]));
        assert_eq!(ifd.get_u32_vec(258), Some(vec![8, 8, 8]));
        // Absent tag is None (distinguishes the real coercion from a constant Some/None).
        assert_eq!(ifd.get_u32_vec(999), None);
    }
}
