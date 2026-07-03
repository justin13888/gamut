//! A typed view over the GPS sub-IFD (Exif 3.0 §4.6.6).
//!
//! [`GpsInfo`] is a convenience projection of the positioning tags — latitude, longitude, and
//! altitude — not a lossless model of the whole GPS directory. The complete directory is always
//! available as the raw [`gamut_ifd::Ifd`] from [`Exif::gps_ifd`](crate::Exif::gps_ifd); this view
//! lifts the common fields into typed coordinates and decimal-degree accessors.

use gamut_ifd::{Ifd, Value};

use crate::tag::ExifTag;
use crate::value::{Rational, as_text};

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
}
