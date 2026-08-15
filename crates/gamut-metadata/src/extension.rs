//! Data no carrier models, carried alongside the model.

use gamut_exif::Value;

/// The namespace prefix reserved for gamut's own extensions.
///
/// A downstream crate picks a namespace it owns — a reverse-DNS string or a URI — and never one
/// starting with this prefix, so a later gamut release cannot collide with it.
pub const RESERVED_NAMESPACE_PREFIX: &str = "gamut.";

/// A datum none of the facade's carriers models, held verbatim so a downstream typed model
/// round-trips through [`Metadata`](crate::Metadata).
///
/// Extensions are the **last resort**, not a general side table. Data that a carrier can express
/// belongs in that carrier, where it also survives to the file: an unmodelled EXIF tag (MakerNote
/// included) round-trips inside [`Metadata::exif`](crate::Metadata::exif) because
/// [`Exif`](gamut_exif::Exif) retains the raw [`Ifd`](gamut_exif::Ifd); an arbitrary property
/// round-trips inside [`Metadata::xmp`](crate::Metadata::xmp) because the XMP graph is open; an
/// unmodelled ICC element round-trips inside [`Metadata::icc`](crate::Metadata::icc) as
/// `TagData::Raw`. Reach for an extension only when *no* carrier can hold the datum — sensor
/// geometry, container-level facts, a downstream's derived structs.
///
/// **Extensions do not serialize.** They have no carrier by construction, so
/// [`MetadataEmbedder`](crate::MetadataEmbedder) drops them (or refuses, under
/// [`ExtensionPolicy::Reject`](crate::ExtensionPolicy)) and
/// [`MetadataExtractor`](crate::MetadataExtractor) never produces them. They survive a
/// *model* round-trip, not a *carrier* one.
///
/// ```
/// use gamut_metadata::{Metadata, MetadataExtension};
/// use gamut_metadata::exif::Value;
///
/// let ext = MetadataExtension::new("com.example.raw", "BlackLevel", Value::Short(vec![512]));
/// assert_eq!(ext.namespace, "com.example.raw");
///
/// let mut meta = Metadata::default();
/// meta.extensions.push(ext);
/// assert_eq!(
///     meta.extension("com.example.raw", "BlackLevel"),
///     Some(&Value::Short(vec![512]))
/// );
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct MetadataExtension {
    /// The namespace owning [`key`](Self::key) — a reverse-DNS string or URI the downstream
    /// controls, e.g. `"com.rawshift.dng"`. Namespaces starting with
    /// [`RESERVED_NAMESPACE_PREFIX`] are reserved for gamut.
    pub namespace: String,
    /// The key within [`namespace`](Self::namespace). Unique per namespace: setting an existing
    /// key through [`Metadata::set_extension`](crate::Metadata::set_extension) replaces its value.
    pub key: String,
    /// The value, in the shared TIFF/IFD value model gamut's metadata crates already use
    /// ([`gamut_exif::Value`], re-exported as `gamut_metadata::exif::Value`). Use
    /// [`Value::Undefined`] for a payload with no typed shape.
    pub value: Value,
}

impl MetadataExtension {
    /// Creates an extension in `namespace` binding `key` to `value`.
    #[must_use]
    pub fn new(namespace: impl Into<String>, key: impl Into<String>, value: Value) -> Self {
        Self {
            namespace: namespace.into(),
            key: key.into(),
            value,
        }
    }

    /// Whether this extension sits in a namespace [reserved](RESERVED_NAMESPACE_PREFIX) for gamut.
    ///
    /// A downstream can assert on this to catch a namespace it does not own.
    #[must_use]
    pub fn is_reserved(&self) -> bool {
        self.namespace.starts_with(RESERVED_NAMESPACE_PREFIX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_stores_the_namespaced_binding() {
        let ext = MetadataExtension::new("com.example", "Key", Value::Long(vec![7]));
        assert_eq!(ext.namespace, "com.example");
        assert_eq!(ext.key, "Key");
        assert_eq!(ext.value, Value::Long(vec![7]));
    }

    #[test]
    fn is_reserved_matches_only_the_gamut_prefix() {
        assert!(MetadataExtension::new("gamut.heic", "k", Value::Long(vec![1])).is_reserved());
        // A namespace merely *containing* the prefix is not reserved — only a leading one is.
        assert!(!MetadataExtension::new("com.gamut.heic", "k", Value::Long(vec![1])).is_reserved());
        assert!(!MetadataExtension::new("com.example", "k", Value::Long(vec![1])).is_reserved());
    }
}
