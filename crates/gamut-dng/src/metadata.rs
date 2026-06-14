//! Optional metadata embedded in a DNG: an EXIF sub-IFD plus XMP / IPTC / ICC blocks.
//!
//! XMP, IPTC, and ICC are carried opaquely (callers supply the already-serialised bytes — gamut's
//! dedicated `gamut-xmp`/`gamut-iptc`/`gamut-icc` crates produce them). EXIF capture settings are a
//! small typed set written into an `ExifIFD` sub-IFD. The encoder writes whatever is present.

use gamut_ifd::{Ifd, Value};

use crate::tags;

/// Common EXIF capture settings, written into the DNG's `ExifIFD` sub-IFD.
///
/// All fields are optional; only those set are emitted. Rationals are `(numerator, denominator)`.
#[derive(Debug, Clone, Default)]
pub struct ExifMetadata {
    /// `ExposureTime` in seconds.
    pub exposure_time: Option<(u32, u32)>,
    /// `FNumber` (the lens f-stop).
    pub f_number: Option<(u32, u32)>,
    /// `ISOSpeedRatings`.
    pub iso_speed: Option<u16>,
    /// `DateTimeOriginal` (`YYYY:MM:DD HH:MM:SS`).
    pub date_time_original: Option<String>,
    /// `FocalLength` in millimetres.
    pub focal_length: Option<(u32, u32)>,
}

impl ExifMetadata {
    /// Whether any field is set (an empty EXIF IFD is not worth writing).
    fn is_empty(&self) -> bool {
        self.exposure_time.is_none()
            && self.f_number.is_none()
            && self.iso_speed.is_none()
            && self.date_time_original.is_none()
            && self.focal_length.is_none()
    }

    /// Builds the EXIF sub-IFD directory.
    fn to_ifd(&self) -> Ifd {
        let mut ifd = Ifd::new();
        // ExifVersion is mandatory in an EXIF IFD; "0230" = EXIF 2.3.
        ifd.set(tags::EXIF_VERSION, Value::Undefined(b"0230".to_vec()));
        if let Some(rate) = self.exposure_time {
            ifd.set(tags::EXPOSURE_TIME, Value::Rational(vec![rate]));
        }
        if let Some(f) = self.f_number {
            ifd.set(tags::F_NUMBER, Value::Rational(vec![f]));
        }
        if let Some(iso) = self.iso_speed {
            ifd.set(tags::ISO_SPEED_RATINGS, Value::Short(vec![iso]));
        }
        if let Some(dt) = &self.date_time_original {
            ifd.set(tags::DATE_TIME_ORIGINAL, Value::Ascii(dt.clone()));
        }
        if let Some(fl) = self.focal_length {
            ifd.set(tags::FOCAL_LENGTH, Value::Rational(vec![fl]));
        }
        ifd
    }
}

/// Metadata to embed in a DNG: an EXIF sub-IFD and/or opaque XMP / IPTC / ICC blocks.
#[derive(Debug, Clone, Default)]
pub struct DngMetadata {
    /// EXIF capture settings (written into an `ExifIFD` sub-IFD).
    pub exif: ExifMetadata,
    /// An XMP packet (UTF-8 RDF/XML), stored in the `XMP` tag.
    pub xmp: Option<Vec<u8>>,
    /// IPTC-IIM metadata, stored in the `IPTC/NAA` tag.
    pub iptc: Option<Vec<u8>>,
    /// An ICC profile, stored in the `ICCProfile` tag.
    pub icc: Option<Vec<u8>>,
}

impl DngMetadata {
    /// Whether there is nothing to embed.
    pub(crate) fn is_empty(&self) -> bool {
        self.exif.is_empty() && self.xmp.is_none() && self.iptc.is_none() && self.icc.is_none()
    }

    /// Writes the XMP / IPTC / ICC blocks into `ifd0` and returns the EXIF sub-IFD, if any.
    pub(crate) fn apply(&self, ifd0: &mut Ifd) -> Option<Ifd> {
        if let Some(xmp) = &self.xmp {
            ifd0.set(tags::XMP, Value::Byte(xmp.clone()));
        }
        if let Some(iptc) = &self.iptc {
            ifd0.set(tags::IPTC_NAA, Value::Byte(iptc.clone()));
        }
        if let Some(icc) = &self.icc {
            ifd0.set(tags::ICC_PROFILE, Value::Undefined(icc.clone()));
        }
        if self.exif.is_empty() {
            None
        } else {
            Some(self.exif.to_ifd())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_metadata_writes_nothing() {
        let mut ifd = Ifd::new();
        let meta = DngMetadata::default();
        assert!(meta.is_empty());
        assert!(meta.apply(&mut ifd).is_none());
        assert!(ifd.fields().is_empty());
    }

    #[test]
    fn applies_blocks_and_builds_exif() {
        let mut ifd = Ifd::new();
        let meta = DngMetadata {
            exif: ExifMetadata {
                iso_speed: Some(400),
                exposure_time: Some((1, 250)),
                ..Default::default()
            },
            xmp: Some(b"<x:xmpmeta/>".to_vec()),
            icc: Some(vec![0u8; 8]),
            ..Default::default()
        };
        assert!(!meta.is_empty());
        let exif = meta.apply(&mut ifd).expect("exif IFD");
        assert_eq!(
            ifd.get(tags::XMP),
            Some(&Value::Byte(b"<x:xmpmeta/>".to_vec()))
        );
        assert!(ifd.get(tags::ICC_PROFILE).is_some());
        assert_eq!(
            exif.get(tags::ISO_SPEED_RATINGS),
            Some(&Value::Short(vec![400]))
        );
        assert_eq!(
            exif.get(tags::EXPOSURE_TIME),
            Some(&Value::Rational(vec![(1, 250)]))
        );
        assert!(exif.get(tags::EXIF_VERSION).is_some());
    }

    #[test]
    fn exif_is_empty_only_when_every_field_is_unset() {
        assert!(ExifMetadata::default().is_empty());
        // Each field on its own makes the IFD worth writing — pins that every `&&` term contributes.
        let singles = [
            ExifMetadata {
                exposure_time: Some((1, 250)),
                ..Default::default()
            },
            ExifMetadata {
                f_number: Some((28, 10)),
                ..Default::default()
            },
            ExifMetadata {
                iso_speed: Some(100),
                ..Default::default()
            },
            ExifMetadata {
                date_time_original: Some("2026:06:14 00:00:00".to_owned()),
                ..Default::default()
            },
            ExifMetadata {
                focal_length: Some((50, 1)),
                ..Default::default()
            },
        ];
        for (i, exif) in singles.iter().enumerate() {
            assert!(!exif.is_empty(), "field {i} alone must be non-empty");
        }
    }

    #[test]
    fn dng_is_empty_only_when_every_block_is_unset() {
        assert!(DngMetadata::default().is_empty());
        let singles = [
            DngMetadata {
                exif: ExifMetadata {
                    iso_speed: Some(100),
                    ..Default::default()
                },
                ..Default::default()
            },
            DngMetadata {
                xmp: Some(vec![1]),
                ..Default::default()
            },
            DngMetadata {
                iptc: Some(vec![1]),
                ..Default::default()
            },
            DngMetadata {
                icc: Some(vec![1]),
                ..Default::default()
            },
        ];
        for (i, meta) in singles.iter().enumerate() {
            assert!(!meta.is_empty(), "block {i} alone must be non-empty");
        }
    }
}
