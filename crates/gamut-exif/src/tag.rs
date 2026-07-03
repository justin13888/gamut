//! The EXIF tag dictionary and the directories tags live in.
//!
//! [`IfdKind`] classifies which directory a tag belongs to; [`ExifTag`] names the standard Exif 3.0
//! tags. The catalogue is generated from a single table (the [`exif_tags!`] invocation), so a tag's
//! id, home directory, and canonical name can never drift apart. It intentionally covers the
//! **standard** CIPA DC-008 tags, not exiftool's full vendor breadth — unknown and MakerNote tags
//! still round-trip losslessly because [`Exif`](crate::Exif) retains the raw [`gamut_ifd::Ifd`].

/// Which IFD a tag belongs to.
///
/// The same 16-bit tag number can mean different things in different directories (e.g. `0x0001` is
/// `GPSLatitudeRef` in [`IfdKind::Gps`] but `InteroperabilityIndex` in [`IfdKind::Interop`]), so a
/// tag is only fully identified by the pair (`IfdKind`, id).
///
/// The 1st IFD ([`IfdKind::Thumbnail`]) reuses the 0th IFD's TIFF baseline tags rather than
/// defining its own, so no [`ExifTag`] is classified under `Thumbnail`; [`ExifTag::from_id`]
/// resolves a `Thumbnail` lookup against the [`IfdKind::Image`] tags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum IfdKind {
    /// The 0th IFD — primary-image / TIFF tags (Make, Model, Orientation, resolution, …).
    Image,
    /// The Exif sub-IFD — capture parameters (exposure, aperture, ISO, lens, …).
    Exif,
    /// The GPS sub-IFD — positioning data.
    Gps,
    /// The Interoperability sub-IFD — interoperability identification.
    Interop,
    /// The 1st IFD — the embedded thumbnail's tags (shares the 0th IFD's tag definitions).
    Thumbnail,
}

/// Generates the [`ExifTag`] enum and its `tag_id` / `ifd` / `name` / `ALL` accessors from one
/// table of `Variant => (IfdKind, id, "CanonicalName")` rows, keeping them in lock-step.
macro_rules! exif_tags {
    ($($variant:ident => ($ifd:ident, $id:expr, $name:expr)),+ $(,)?) => {
        /// A standard EXIF tag.
        ///
        /// `#[non_exhaustive]` so tags can be added post-1.0 without a breaking change. Each variant
        /// maps to a 16-bit on-disk tag number ([`ExifTag::tag_id`]) within its home directory
        /// ([`ExifTag::ifd`]); [`ExifTag::name`] gives the canonical CIPA DC-008 name.
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[non_exhaustive]
        pub enum ExifTag {
            $(#[doc = $name] $variant,)+
        }

        impl ExifTag {
            /// Every catalogued tag, in declaration order.
            pub const ALL: &'static [ExifTag] = &[$(ExifTag::$variant),+];

            /// The 16-bit on-disk tag number.
            #[must_use]
            pub const fn tag_id(self) -> u16 {
                match self { $(ExifTag::$variant => $id),+ }
            }

            /// The directory this tag belongs to.
            #[must_use]
            pub const fn ifd(self) -> IfdKind {
                match self { $(ExifTag::$variant => IfdKind::$ifd),+ }
            }

            /// The canonical CIPA DC-008 tag name (stable across variant renames).
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self { $(ExifTag::$variant => $name),+ }
            }
        }
    };
}

impl ExifTag {
    /// Looks up a tag by its directory and on-disk id, or `None` if it is not a catalogued standard
    /// tag.
    ///
    /// A [`IfdKind::Thumbnail`] lookup resolves against the [`IfdKind::Image`] tags, since the 1st
    /// IFD reuses the 0th IFD's TIFF baseline definitions.
    #[must_use]
    pub fn from_id(ifd: IfdKind, id: u16) -> Option<ExifTag> {
        let ifd = match ifd {
            IfdKind::Thumbnail => IfdKind::Image,
            other => other,
        };
        ExifTag::ALL
            .iter()
            .copied()
            .find(|t| t.tag_id() == id && t.ifd() == ifd)
    }
}

exif_tags! {
    // ---- 0th IFD (TIFF baseline + EXIF-added image attributes); also used by the 1st IFD ----
    ImageWidth => (Image, 0x0100, "ImageWidth"),
    ImageLength => (Image, 0x0101, "ImageLength"),
    BitsPerSample => (Image, 0x0102, "BitsPerSample"),
    Compression => (Image, 0x0103, "Compression"),
    PhotometricInterpretation => (Image, 0x0106, "PhotometricInterpretation"),
    ImageDescription => (Image, 0x010E, "ImageDescription"),
    Make => (Image, 0x010F, "Make"),
    Model => (Image, 0x0110, "Model"),
    StripOffsets => (Image, 0x0111, "StripOffsets"),
    Orientation => (Image, 0x0112, "Orientation"),
    SamplesPerPixel => (Image, 0x0115, "SamplesPerPixel"),
    RowsPerStrip => (Image, 0x0116, "RowsPerStrip"),
    StripByteCounts => (Image, 0x0117, "StripByteCounts"),
    XResolution => (Image, 0x011A, "XResolution"),
    YResolution => (Image, 0x011B, "YResolution"),
    PlanarConfiguration => (Image, 0x011C, "PlanarConfiguration"),
    ResolutionUnit => (Image, 0x0128, "ResolutionUnit"),
    TransferFunction => (Image, 0x012D, "TransferFunction"),
    Software => (Image, 0x0131, "Software"),
    DateTime => (Image, 0x0132, "DateTime"),
    Artist => (Image, 0x013B, "Artist"),
    WhitePoint => (Image, 0x013E, "WhitePoint"),
    PrimaryChromaticities => (Image, 0x013F, "PrimaryChromaticities"),
    JpegInterchangeFormat => (Image, 0x0201, "JPEGInterchangeFormat"),
    JpegInterchangeFormatLength => (Image, 0x0202, "JPEGInterchangeFormatLength"),
    YCbCrCoefficients => (Image, 0x0211, "YCbCrCoefficients"),
    YCbCrSubSampling => (Image, 0x0212, "YCbCrSubSampling"),
    YCbCrPositioning => (Image, 0x0213, "YCbCrPositioning"),
    ReferenceBlackWhite => (Image, 0x0214, "ReferenceBlackWhite"),
    Xmp => (Image, 0x02BC, "ApplicationNotes"),
    Copyright => (Image, 0x8298, "Copyright"),
    IptcNaa => (Image, 0x83BB, "IPTC-NAA"),
    InterColorProfile => (Image, 0x8773, "InterColorProfile"),
    Rating => (Image, 0x4746, "Rating"),
    RatingPercent => (Image, 0x4749, "RatingPercent"),

    // ---- Exif sub-IFD (capture parameters) ----
    ExposureTime => (Exif, 0x829A, "ExposureTime"),
    FNumber => (Exif, 0x829D, "FNumber"),
    ExposureProgram => (Exif, 0x8822, "ExposureProgram"),
    SpectralSensitivity => (Exif, 0x8824, "SpectralSensitivity"),
    PhotographicSensitivity => (Exif, 0x8827, "PhotographicSensitivity"),
    Oecf => (Exif, 0x8828, "OECF"),
    SensitivityType => (Exif, 0x8830, "SensitivityType"),
    StandardOutputSensitivity => (Exif, 0x8831, "StandardOutputSensitivity"),
    RecommendedExposureIndex => (Exif, 0x8832, "RecommendedExposureIndex"),
    IsoSpeed => (Exif, 0x8833, "ISOSpeed"),
    IsoSpeedLatitudeYyy => (Exif, 0x8834, "ISOSpeedLatitudeyyy"),
    IsoSpeedLatitudeZzz => (Exif, 0x8835, "ISOSpeedLatitudezzz"),
    ExifVersion => (Exif, 0x9000, "ExifVersion"),
    DateTimeOriginal => (Exif, 0x9003, "DateTimeOriginal"),
    DateTimeDigitized => (Exif, 0x9004, "DateTimeDigitized"),
    OffsetTime => (Exif, 0x9010, "OffsetTime"),
    OffsetTimeOriginal => (Exif, 0x9011, "OffsetTimeOriginal"),
    OffsetTimeDigitized => (Exif, 0x9012, "OffsetTimeDigitized"),
    ComponentsConfiguration => (Exif, 0x9101, "ComponentsConfiguration"),
    CompressedBitsPerPixel => (Exif, 0x9102, "CompressedBitsPerPixel"),
    ShutterSpeedValue => (Exif, 0x9201, "ShutterSpeedValue"),
    ApertureValue => (Exif, 0x9202, "ApertureValue"),
    BrightnessValue => (Exif, 0x9203, "BrightnessValue"),
    ExposureBiasValue => (Exif, 0x9204, "ExposureBiasValue"),
    MaxApertureValue => (Exif, 0x9205, "MaxApertureValue"),
    SubjectDistance => (Exif, 0x9206, "SubjectDistance"),
    MeteringMode => (Exif, 0x9207, "MeteringMode"),
    LightSource => (Exif, 0x9208, "LightSource"),
    Flash => (Exif, 0x9209, "Flash"),
    FocalLength => (Exif, 0x920A, "FocalLength"),
    SubjectArea => (Exif, 0x9214, "SubjectArea"),
    MakerNote => (Exif, 0x927C, "MakerNote"),
    UserComment => (Exif, 0x9286, "UserComment"),
    SubSecTime => (Exif, 0x9290, "SubSecTime"),
    SubSecTimeOriginal => (Exif, 0x9291, "SubSecTimeOriginal"),
    SubSecTimeDigitized => (Exif, 0x9292, "SubSecTimeDigitized"),
    Temperature => (Exif, 0x9400, "Temperature"),
    Humidity => (Exif, 0x9401, "Humidity"),
    Pressure => (Exif, 0x9402, "Pressure"),
    WaterDepth => (Exif, 0x9403, "WaterDepth"),
    Acceleration => (Exif, 0x9404, "Acceleration"),
    CameraElevationAngle => (Exif, 0x9405, "CameraElevationAngle"),
    FlashpixVersion => (Exif, 0xA000, "FlashpixVersion"),
    ColorSpace => (Exif, 0xA001, "ColorSpace"),
    PixelXDimension => (Exif, 0xA002, "PixelXDimension"),
    PixelYDimension => (Exif, 0xA003, "PixelYDimension"),
    RelatedSoundFile => (Exif, 0xA004, "RelatedSoundFile"),
    FlashEnergy => (Exif, 0xA20B, "FlashEnergy"),
    SpatialFrequencyResponse => (Exif, 0xA20C, "SpatialFrequencyResponse"),
    FocalPlaneXResolution => (Exif, 0xA20E, "FocalPlaneXResolution"),
    FocalPlaneYResolution => (Exif, 0xA20F, "FocalPlaneYResolution"),
    FocalPlaneResolutionUnit => (Exif, 0xA210, "FocalPlaneResolutionUnit"),
    SubjectLocation => (Exif, 0xA214, "SubjectLocation"),
    ExposureIndex => (Exif, 0xA215, "ExposureIndex"),
    SensingMethod => (Exif, 0xA217, "SensingMethod"),
    FileSource => (Exif, 0xA300, "FileSource"),
    SceneType => (Exif, 0xA301, "SceneType"),
    CfaPattern => (Exif, 0xA302, "CFAPattern"),
    CustomRendered => (Exif, 0xA401, "CustomRendered"),
    ExposureMode => (Exif, 0xA402, "ExposureMode"),
    WhiteBalance => (Exif, 0xA403, "WhiteBalance"),
    DigitalZoomRatio => (Exif, 0xA404, "DigitalZoomRatio"),
    FocalLengthIn35mmFilm => (Exif, 0xA405, "FocalLengthIn35mmFilm"),
    SceneCaptureType => (Exif, 0xA406, "SceneCaptureType"),
    GainControl => (Exif, 0xA407, "GainControl"),
    Contrast => (Exif, 0xA408, "Contrast"),
    Saturation => (Exif, 0xA409, "Saturation"),
    Sharpness => (Exif, 0xA40A, "Sharpness"),
    DeviceSettingDescription => (Exif, 0xA40B, "DeviceSettingDescription"),
    SubjectDistanceRange => (Exif, 0xA40C, "SubjectDistanceRange"),
    ImageUniqueId => (Exif, 0xA420, "ImageUniqueID"),
    CameraOwnerName => (Exif, 0xA430, "CameraOwnerName"),
    BodySerialNumber => (Exif, 0xA431, "BodySerialNumber"),
    LensSpecification => (Exif, 0xA432, "LensSpecification"),
    LensMake => (Exif, 0xA433, "LensMake"),
    LensModel => (Exif, 0xA434, "LensModel"),
    LensSerialNumber => (Exif, 0xA435, "LensSerialNumber"),
    Gamma => (Exif, 0xA500, "Gamma"),
    CompositeImage => (Exif, 0xA460, "CompositeImage"),
    SourceImageNumberOfCompositeImage => (Exif, 0xA461, "SourceImageNumberOfCompositeImage"),
    SourceExposureTimesOfCompositeImage => (Exif, 0xA462, "SourceExposureTimesOfCompositeImage"),

    // ---- GPS sub-IFD ----
    GpsVersionId => (Gps, 0x0000, "GPSVersionID"),
    GpsLatitudeRef => (Gps, 0x0001, "GPSLatitudeRef"),
    GpsLatitude => (Gps, 0x0002, "GPSLatitude"),
    GpsLongitudeRef => (Gps, 0x0003, "GPSLongitudeRef"),
    GpsLongitude => (Gps, 0x0004, "GPSLongitude"),
    GpsAltitudeRef => (Gps, 0x0005, "GPSAltitudeRef"),
    GpsAltitude => (Gps, 0x0006, "GPSAltitude"),
    GpsTimeStamp => (Gps, 0x0007, "GPSTimeStamp"),
    GpsSatellites => (Gps, 0x0008, "GPSSatellites"),
    GpsStatus => (Gps, 0x0009, "GPSStatus"),
    GpsMeasureMode => (Gps, 0x000A, "GPSMeasureMode"),
    GpsDop => (Gps, 0x000B, "GPSDOP"),
    GpsSpeedRef => (Gps, 0x000C, "GPSSpeedRef"),
    GpsSpeed => (Gps, 0x000D, "GPSSpeed"),
    GpsTrackRef => (Gps, 0x000E, "GPSTrackRef"),
    GpsTrack => (Gps, 0x000F, "GPSTrack"),
    GpsImgDirectionRef => (Gps, 0x0010, "GPSImgDirectionRef"),
    GpsImgDirection => (Gps, 0x0011, "GPSImgDirection"),
    GpsMapDatum => (Gps, 0x0012, "GPSMapDatum"),
    GpsDestLatitudeRef => (Gps, 0x0013, "GPSDestLatitudeRef"),
    GpsDestLatitude => (Gps, 0x0014, "GPSDestLatitude"),
    GpsDestLongitudeRef => (Gps, 0x0015, "GPSDestLongitudeRef"),
    GpsDestLongitude => (Gps, 0x0016, "GPSDestLongitude"),
    GpsDestBearingRef => (Gps, 0x0017, "GPSDestBearingRef"),
    GpsDestBearing => (Gps, 0x0018, "GPSDestBearing"),
    GpsDestDistanceRef => (Gps, 0x0019, "GPSDestDistanceRef"),
    GpsDestDistance => (Gps, 0x001A, "GPSDestDistance"),
    GpsProcessingMethod => (Gps, 0x001B, "GPSProcessingMethod"),
    GpsAreaInformation => (Gps, 0x001C, "GPSAreaInformation"),
    GpsDateStamp => (Gps, 0x001D, "GPSDateStamp"),
    GpsDifferential => (Gps, 0x001E, "GPSDifferential"),
    GpsHPositioningError => (Gps, 0x001F, "GPSHPositioningError"),

    // ---- Interoperability sub-IFD ----
    InteroperabilityIndex => (Interop, 0x0001, "InteroperabilityIndex"),
    InteroperabilityVersion => (Interop, 0x0002, "InteroperabilityVersion"),
    RelatedImageFileFormat => (Interop, 0x1000, "RelatedImageFileFormat"),
    RelatedImageWidth => (Interop, 0x1001, "RelatedImageWidth"),
    RelatedImageLength => (Interop, 0x1002, "RelatedImageLength"),
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn tag_ids_are_unique_within_each_ifd() {
        // Two tags sharing an (ifd, id) would make the catalogue ambiguous and break from_id.
        let mut seen = HashSet::new();
        for &tag in ExifTag::ALL {
            assert!(
                seen.insert((tag.ifd(), tag.tag_id())),
                "duplicate (ifd, id) for {tag:?}"
            );
        }
    }

    #[test]
    fn from_id_round_trips_every_tag() {
        for &tag in ExifTag::ALL {
            assert_eq!(ExifTag::from_id(tag.ifd(), tag.tag_id()), Some(tag));
        }
    }

    #[test]
    fn same_id_disambiguated_by_ifd() {
        // 0x0001 is GPSLatitudeRef in GPS but InteroperabilityIndex in Interop.
        assert_eq!(
            ExifTag::from_id(IfdKind::Gps, 0x0001),
            Some(ExifTag::GpsLatitudeRef)
        );
        assert_eq!(
            ExifTag::from_id(IfdKind::Interop, 0x0001),
            Some(ExifTag::InteroperabilityIndex)
        );
        assert_eq!(ExifTag::GpsLatitudeRef.tag_id(), 0x0001);
        assert_eq!(ExifTag::InteroperabilityIndex.tag_id(), 0x0001);
    }

    #[test]
    fn thumbnail_resolves_against_image_baseline() {
        // The 1st IFD reuses the 0th IFD's tag definitions.
        assert_eq!(
            ExifTag::from_id(IfdKind::Thumbnail, 0x0103),
            Some(ExifTag::Compression)
        );
        assert_eq!(
            ExifTag::from_id(IfdKind::Thumbnail, 0x0201),
            Some(ExifTag::JpegInterchangeFormat)
        );
    }

    #[test]
    fn unknown_ids_are_none() {
        assert_eq!(ExifTag::from_id(IfdKind::Image, 0xFFFF), None);
        // A GPS id looked up in the Exif IFD does not resolve.
        assert_eq!(ExifTag::from_id(IfdKind::Exif, 0x0002), None);
    }

    #[test]
    fn name_and_ids_pin_representative_tags() {
        assert_eq!(ExifTag::FNumber.tag_id(), 0x829D);
        assert_eq!(ExifTag::FNumber.ifd(), IfdKind::Exif);
        assert_eq!(ExifTag::FNumber.name(), "FNumber");
        assert_eq!(ExifTag::Make.name(), "Make");
        assert_eq!(ExifTag::IptcNaa.name(), "IPTC-NAA");
        assert_eq!(ExifTag::GpsLatitude.tag_id(), 0x0002);
        assert!(ExifTag::ALL.len() > 140);
    }
}
