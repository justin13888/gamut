//! A typed view over the GPS sub-IFD (Exif 3.0 §4.6.6).
//!
//! [`GpsInfo`] is a convenience projection of the positioning tags — latitude, longitude, and
//! altitude — not a lossless model of the whole GPS directory. The complete directory is always
//! available as the raw [`gamut_ifd::Ifd`] from [`Exif::gps_ifd`](crate::Exif::gps_ifd); this view
//! lifts the common fields into typed coordinates and decimal-degree accessors.

use gamut_ifd::{Ifd, Value};

use crate::tag::ExifTag;
use crate::value::{Rational, as_text};

/// Errors converting EXIF GPS fields into `geocoordinates` types.
#[cfg(feature = "geocoordinates")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum GpsConversionError {
    /// Latitude is required for a complete position.
    #[error("GPS latitude is missing")]
    MissingLatitude,
    /// Longitude is required for a complete position.
    #[error("GPS longitude is missing")]
    MissingLongitude,
    /// Latitude used an east/west reference instead of north/south.
    #[error("GPS latitude reference must be N or S")]
    InvalidLatitudeReference,
    /// Longitude used a north/south reference instead of east/west.
    #[error("GPS longitude reference must be E or W")]
    InvalidLongitudeReference,
    /// A rational component had a zero denominator.
    #[error("GPS {0} has a zero denominator")]
    ZeroDenominator(&'static str),
    /// A DMS coordinate used minutes or seconds outside `[0, 60)`.
    #[error("GPS {0} has invalid degrees/minutes/seconds components")]
    InvalidDms(&'static str),
    /// The resulting latitude or longitude is outside the WGS-84 angular domain.
    #[error("GPS position is outside the valid latitude/longitude range")]
    OutOfRange,
}

/// The positioning data from the GPS sub-IFD, as typed latitude/longitude/altitude.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpsInfo {
    /// Latitude (paired with its N/S reference), if present.
    pub latitude: Option<GpsCoordinate>,
    /// Longitude (paired with its E/W reference), if present.
    pub longitude: Option<GpsCoordinate>,
    /// Altitude relative to sea level, if present.
    pub altitude: Option<GpsAltitude>,
}

/// A GPS coordinate: degrees/minutes/seconds as rationals, plus the hemisphere reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpsCoordinate {
    /// Degrees.
    pub degrees: Rational,
    /// Minutes.
    pub minutes: Rational,
    /// Seconds.
    pub seconds: Rational,
    /// The hemisphere reference (`N`/`S` for latitude, `E`/`W` for longitude).
    pub reference: GpsReference,
}

/// An altitude: metres from sea level, with the sign carried by `below_sea_level`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpsAltitude {
    /// The magnitude in metres.
    pub meters: Rational,
    /// Whether the altitude is below sea level (`GPSAltitudeRef` = 1).
    pub below_sea_level: bool,
}

/// The hemisphere reference of a GPS coordinate (`GPSLatitudeRef` / `GPSLongitudeRef`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GpsReference {
    /// `N` — northern latitude.
    North,
    /// `S` — southern latitude.
    South,
    /// `E` — eastern longitude.
    East,
    /// `W` — western longitude.
    West,
}

impl GpsReference {
    /// Parses the single-letter reference string (`"N"`/`"S"`/`"E"`/`"W"`).
    fn parse(s: &str) -> Option<Self> {
        match s.trim_end_matches('\0').chars().next()? {
            'N' | 'n' => Some(GpsReference::North),
            'S' | 's' => Some(GpsReference::South),
            'E' | 'e' => Some(GpsReference::East),
            'W' | 'w' => Some(GpsReference::West),
            _ => None,
        }
    }

    /// The single-letter reference string.
    fn as_str(self) -> &'static str {
        match self {
            GpsReference::North => "N",
            GpsReference::South => "S",
            GpsReference::East => "E",
            GpsReference::West => "W",
        }
    }

    /// `-1.0` for the southern/western hemispheres, `+1.0` otherwise.
    fn sign(self) -> f64 {
        match self {
            GpsReference::South | GpsReference::West => -1.0,
            GpsReference::North | GpsReference::East => 1.0,
        }
    }
}

impl GpsCoordinate {
    /// The coordinate as signed decimal degrees, or `None` if any rational has a zero denominator.
    #[must_use]
    pub fn to_degrees(self) -> Option<f64> {
        let d = self.degrees.to_f64()?;
        let m = self.minutes.to_f64()?;
        let s = self.seconds.to_f64()?;
        Some(self.reference.sign() * (d + m / 60.0 + s / 3600.0))
    }
}

impl GpsInfo {
    /// Lifts the positioning tags out of a GPS sub-IFD, or `None` if none are present.
    #[must_use]
    pub fn from_ifd(ifd: &Ifd) -> Option<GpsInfo> {
        let latitude = coordinate(ifd, ExifTag::GpsLatitude, ExifTag::GpsLatitudeRef);
        let longitude = coordinate(ifd, ExifTag::GpsLongitude, ExifTag::GpsLongitudeRef);
        let altitude = altitude(ifd);
        if latitude.is_none() && longitude.is_none() && altitude.is_none() {
            None
        } else {
            Some(GpsInfo {
                latitude,
                longitude,
                altitude,
            })
        }
    }

    /// Builds a minimal, spec-valid GPS sub-IFD from this view.
    ///
    /// This carries only the latitude/longitude/altitude tags plus the mandatory `GPSVersionID`
    /// (2.3.0.0); it is not the inverse of [`from_ifd`](Self::from_ifd) for a directory that held
    /// other GPS tags (use the raw [`Exif::gps_ifd`](crate::Exif::gps_ifd) to preserve those).
    #[must_use]
    pub fn to_ifd(&self) -> Ifd {
        let mut ifd = Ifd::new();
        ifd.set(
            ExifTag::GpsVersionId.tag_id(),
            Value::Byte(vec![2, 3, 0, 0]),
        );
        if let Some(lat) = self.latitude {
            set_coordinate(&mut ifd, lat, ExifTag::GpsLatitude, ExifTag::GpsLatitudeRef);
        }
        if let Some(lon) = self.longitude {
            set_coordinate(
                &mut ifd,
                lon,
                ExifTag::GpsLongitude,
                ExifTag::GpsLongitudeRef,
            );
        }
        if let Some(alt) = self.altitude {
            ifd.set(
                ExifTag::GpsAltitudeRef.tag_id(),
                Value::Byte(vec![u8::from(alt.below_sea_level)]),
            );
            ifd.set(
                ExifTag::GpsAltitude.tag_id(),
                Value::Rational(vec![alt.meters.into()]),
            );
        }
        ifd
    }

    /// Latitude in signed decimal degrees (negative for the southern hemisphere).
    #[must_use]
    pub fn latitude_deg(&self) -> Option<f64> {
        self.latitude.and_then(GpsCoordinate::to_degrees)
    }

    /// Longitude in signed decimal degrees (negative for the western hemisphere).
    #[must_use]
    pub fn longitude_deg(&self) -> Option<f64> {
        self.longitude.and_then(GpsCoordinate::to_degrees)
    }

    /// Altitude in signed metres (negative below sea level).
    #[must_use]
    pub fn altitude_m(&self) -> Option<f64> {
        self.altitude.and_then(|a| {
            a.meters
                .to_f64()
                .map(|m| if a.below_sea_level { -m } else { m })
        })
    }
}

#[cfg(feature = "geocoordinates")]
impl TryFrom<GpsInfo> for geocoordinates::Wgs84 {
    type Error = GpsConversionError;

    /// Converts a complete, valid EXIF GPS position to decimal-degree WGS-84.
    ///
    /// EXIF altitude is intentionally omitted because [`geocoordinates::Wgs84`] is a 2D datum
    /// newtype. Convert to [`geocoordinates::Coordinate`] to preserve it.
    fn try_from(value: GpsInfo) -> Result<Self, Self::Error> {
        let coordinate = coordinate_without_height(&value)?;
        Ok(geocoordinates::Wgs84::new(coordinate.lat, coordinate.lon))
    }
}

#[cfg(feature = "geocoordinates")]
impl TryFrom<GpsInfo> for geocoordinates::Coordinate {
    type Error = GpsConversionError;

    /// Converts a complete, valid EXIF GPS position to a WGS-84 coordinate, preserving EXIF
    /// sea-level altitude as an orthometric height when present.
    fn try_from(value: GpsInfo) -> Result<Self, Self::Error> {
        let mut coordinate = coordinate_without_height(&value)?;
        if let Some(altitude) = value.altitude {
            let meters = altitude
                .meters
                .to_f64()
                .ok_or(GpsConversionError::ZeroDenominator("altitude"))?;
            let signed = if altitude.below_sea_level {
                -meters
            } else {
                meters
            };
            coordinate = coordinate.with_height(geocoordinates::Height::Orthometric(signed));
        }
        coordinate
            .validate()
            .map_err(|_| GpsConversionError::OutOfRange)?;
        Ok(coordinate)
    }
}

#[cfg(feature = "geocoordinates")]
fn coordinate_without_height(
    value: &GpsInfo,
) -> Result<geocoordinates::Coordinate, GpsConversionError> {
    let latitude = value.latitude.ok_or(GpsConversionError::MissingLatitude)?;
    let longitude = value
        .longitude
        .ok_or(GpsConversionError::MissingLongitude)?;
    let lat = coordinate_degrees(latitude, true)?;
    let lon = coordinate_degrees(longitude, false)?;
    let coordinate = geocoordinates::Coordinate::wgs84(lat, lon);
    coordinate
        .validate()
        .map_err(|_| GpsConversionError::OutOfRange)?;
    Ok(coordinate)
}

#[cfg(feature = "geocoordinates")]
fn coordinate_degrees(
    coordinate: GpsCoordinate,
    latitude: bool,
) -> Result<f64, GpsConversionError> {
    let sign = match (latitude, coordinate.reference) {
        (true, GpsReference::North) | (false, GpsReference::East) => 1.0,
        (true, GpsReference::South) | (false, GpsReference::West) => -1.0,
        (true, GpsReference::East | GpsReference::West) => {
            return Err(GpsConversionError::InvalidLatitudeReference);
        }
        (false, GpsReference::North | GpsReference::South) => {
            return Err(GpsConversionError::InvalidLongitudeReference);
        }
    };
    let axis = if latitude { "latitude" } else { "longitude" };
    let degrees = coordinate
        .degrees
        .to_f64()
        .ok_or(GpsConversionError::ZeroDenominator(axis))?;
    let minutes = coordinate
        .minutes
        .to_f64()
        .ok_or(GpsConversionError::ZeroDenominator(axis))?;
    let seconds = coordinate
        .seconds
        .to_f64()
        .ok_or(GpsConversionError::ZeroDenominator(axis))?;
    if minutes >= 60.0 || seconds >= 60.0 {
        return Err(GpsConversionError::InvalidDms(axis));
    }
    Ok(sign * (degrees + minutes / 60.0 + seconds / 3600.0))
}

/// Reads a degrees/minutes/seconds coordinate and its reference from `ifd`.
fn coordinate(ifd: &Ifd, value_tag: ExifTag, ref_tag: ExifTag) -> Option<GpsCoordinate> {
    let dms = match ifd.get(value_tag.tag_id())? {
        Value::Rational(r) if r.len() >= 3 => r,
        _ => return None,
    };
    let reference = GpsReference::parse(as_text(ifd.get(ref_tag.tag_id())?)?)?;
    Some(GpsCoordinate {
        degrees: dms[0].into(),
        minutes: dms[1].into(),
        seconds: dms[2].into(),
        reference,
    })
}

/// Writes a coordinate's reference letter and its three DMS rationals into `ifd`.
fn set_coordinate(ifd: &mut Ifd, coord: GpsCoordinate, value_tag: ExifTag, ref_tag: ExifTag) {
    ifd.set(
        ref_tag.tag_id(),
        Value::Ascii(coord.reference.as_str().to_owned()),
    );
    ifd.set(
        value_tag.tag_id(),
        Value::Rational(vec![
            coord.degrees.into(),
            coord.minutes.into(),
            coord.seconds.into(),
        ]),
    );
}

/// Reads the altitude and its above/below-sea-level reference from `ifd`.
fn altitude(ifd: &Ifd) -> Option<GpsAltitude> {
    let meters = match ifd.get(ExifTag::GpsAltitude.tag_id())? {
        Value::Rational(r) => (*r.first()?).into(),
        _ => return None,
    };
    // GPSAltitudeRef: 0 = above sea level (the default when absent), 1 = below.
    let below_sea_level = ifd.get_u32(ExifTag::GpsAltitudeRef.tag_id()).unwrap_or(0) == 1;
    Some(GpsAltitude {
        meters,
        below_sea_level,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_ifd() -> Ifd {
        let mut ifd = Ifd::new();
        // 48°51'29.6" N, 2°21'02.4" E, 35 m above sea level (approx. Paris).
        ifd.set(ExifTag::GpsLatitudeRef.tag_id(), Value::Ascii("N".into()));
        ifd.set(
            ExifTag::GpsLatitude.tag_id(),
            Value::Rational(vec![(48, 1), (51, 1), (296, 10)]),
        );
        ifd.set(ExifTag::GpsLongitudeRef.tag_id(), Value::Ascii("E".into()));
        ifd.set(
            ExifTag::GpsLongitude.tag_id(),
            Value::Rational(vec![(2, 1), (21, 1), (24, 10)]),
        );
        ifd.set(ExifTag::GpsAltitudeRef.tag_id(), Value::Byte(vec![0]));
        ifd.set(
            ExifTag::GpsAltitude.tag_id(),
            Value::Rational(vec![(35, 1)]),
        );
        ifd
    }

    #[test]
    fn parses_coordinates_and_altitude() {
        let gps = GpsInfo::from_ifd(&sample_ifd()).expect("gps present");
        assert_eq!(gps.latitude.unwrap().reference, GpsReference::North);
        assert_eq!(gps.longitude.unwrap().reference, GpsReference::East);
        assert!((gps.latitude_deg().unwrap() - 48.858_222).abs() < 1e-5);
        assert!((gps.longitude_deg().unwrap() - 2.350_666).abs() < 1e-5);
        assert_eq!(gps.altitude_m(), Some(35.0));
    }

    #[test]
    fn southern_and_below_sea_level_are_negative() {
        let mut ifd = sample_ifd();
        ifd.set(ExifTag::GpsLatitudeRef.tag_id(), Value::Ascii("S".into()));
        ifd.set(ExifTag::GpsAltitudeRef.tag_id(), Value::Byte(vec![1]));
        let gps = GpsInfo::from_ifd(&ifd).expect("gps");
        assert!(gps.latitude_deg().unwrap() < 0.0);
        assert_eq!(gps.altitude_m(), Some(-35.0));
    }

    #[test]
    fn empty_ifd_has_no_gps_info() {
        assert_eq!(GpsInfo::from_ifd(&Ifd::new()), None);
    }

    #[test]
    fn to_ifd_round_trips_the_typed_view() {
        let original = GpsInfo::from_ifd(&sample_ifd()).expect("gps");
        let rebuilt = GpsInfo::from_ifd(&original.to_ifd()).expect("rebuilt gps");
        assert_eq!(rebuilt, original);
        // The rebuilt directory carries the mandatory version tag.
        assert_eq!(
            original.to_ifd().get(ExifTag::GpsVersionId.tag_id()),
            Some(&Value::Byte(vec![2, 3, 0, 0]))
        );
    }

    #[test]
    fn zero_denominator_yields_no_degrees() {
        let coord = GpsCoordinate {
            degrees: Rational { num: 1, den: 0 },
            minutes: Rational { num: 0, den: 1 },
            seconds: Rational { num: 0, den: 1 },
            reference: GpsReference::North,
        };
        assert_eq!(coord.to_degrees(), None);
    }

    #[cfg(feature = "geocoordinates")]
    mod geocoordinates_conversion {
        use geocoordinates::{Coordinate, Crs, Height, Wgs84};

        use super::*;

        fn dms(degrees: u32, minutes: u32, seconds: u32, reference: GpsReference) -> GpsCoordinate {
            GpsCoordinate {
                degrees: Rational {
                    num: degrees,
                    den: 1,
                },
                minutes: Rational {
                    num: minutes,
                    den: 1,
                },
                seconds: Rational {
                    num: seconds,
                    den: 1,
                },
                reference,
            }
        }

        fn position() -> GpsInfo {
            GpsInfo {
                latitude: Some(dms(48, 51, 30, GpsReference::North)),
                longitude: Some(dms(2, 21, 2, GpsReference::East)),
                altitude: Some(GpsAltitude {
                    meters: Rational { num: 35, den: 1 },
                    below_sea_level: false,
                }),
            }
        }

        #[test]
        fn wgs84_conversion_uses_axis_signs_and_drops_altitude() {
            let north_east = Wgs84::try_from(position()).unwrap();
            assert!((north_east.lat - 48.858_333_333_333_334).abs() < 1e-12);
            assert!((north_east.lon - 2.350_555_555_555_555_4).abs() < 1e-12);

            let mut south_west = position();
            south_west.latitude.as_mut().unwrap().reference = GpsReference::South;
            south_west.longitude.as_mut().unwrap().reference = GpsReference::West;
            let converted = Wgs84::try_from(south_west).unwrap();
            assert_eq!(converted.lat, -north_east.lat);
            assert_eq!(converted.lon, -north_east.lon);
        }

        #[test]
        fn coordinate_conversion_preserves_orthometric_altitude() {
            let above = Coordinate::try_from(position()).unwrap();
            assert_eq!(above.crs, Crs::Wgs84);
            assert_eq!(above.height, Some(Height::Orthometric(35.0)));
            above.validate().unwrap();

            let mut below = position();
            below.altitude.as_mut().unwrap().below_sea_level = true;
            assert_eq!(
                Coordinate::try_from(below).unwrap().height,
                Some(Height::Orthometric(-35.0))
            );

            let mut no_altitude = position();
            no_altitude.altitude = None;
            assert_eq!(Coordinate::try_from(no_altitude).unwrap().height, None);
        }

        #[test]
        fn complete_coordinates_and_axis_references_are_required() {
            let mut missing_latitude = position();
            missing_latitude.latitude = None;
            assert_eq!(
                Wgs84::try_from(missing_latitude),
                Err(GpsConversionError::MissingLatitude)
            );

            let mut missing_longitude = position();
            missing_longitude.longitude = None;
            assert_eq!(
                Wgs84::try_from(missing_longitude),
                Err(GpsConversionError::MissingLongitude)
            );

            let mut bad_latitude_ref = position();
            bad_latitude_ref.latitude.as_mut().unwrap().reference = GpsReference::East;
            assert_eq!(
                Wgs84::try_from(bad_latitude_ref),
                Err(GpsConversionError::InvalidLatitudeReference)
            );

            let mut bad_longitude_ref = position();
            bad_longitude_ref.longitude.as_mut().unwrap().reference = GpsReference::North;
            assert_eq!(
                Wgs84::try_from(bad_longitude_ref),
                Err(GpsConversionError::InvalidLongitudeReference)
            );
        }

        #[test]
        fn malformed_rationals_and_dms_are_rejected() {
            let mut zero_denominator = position();
            zero_denominator.latitude.as_mut().unwrap().seconds.den = 0;
            assert_eq!(
                Wgs84::try_from(zero_denominator),
                Err(GpsConversionError::ZeroDenominator("latitude"))
            );

            let mut invalid_dms = position();
            invalid_dms.longitude.as_mut().unwrap().minutes.num = 60;
            assert_eq!(
                Wgs84::try_from(invalid_dms),
                Err(GpsConversionError::InvalidDms("longitude"))
            );

            let mut invalid_altitude = position();
            invalid_altitude.altitude.as_mut().unwrap().meters.den = 0;
            assert_eq!(
                Coordinate::try_from(invalid_altitude),
                Err(GpsConversionError::ZeroDenominator("altitude"))
            );
            assert!(
                Wgs84::try_from(invalid_altitude).is_ok(),
                "the 2D conversion deliberately ignores altitude"
            );
        }

        #[test]
        fn angular_boundaries_are_valid_and_excess_is_rejected() {
            let boundary = GpsInfo {
                latitude: Some(dms(90, 0, 0, GpsReference::South)),
                longitude: Some(dms(180, 0, 0, GpsReference::West)),
                altitude: None,
            };
            assert_eq!(Wgs84::try_from(boundary), Ok(Wgs84::new(-90.0, -180.0)));

            let mut beyond_pole = boundary;
            beyond_pole.latitude = Some(dms(90, 0, 1, GpsReference::North));
            assert_eq!(
                Coordinate::try_from(beyond_pole),
                Err(GpsConversionError::OutOfRange)
            );
        }
    }
}
