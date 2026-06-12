//! The GPS sub-IFD, modelled as typed coordinates.

/// The positioning data from the GPS sub-IFD, lifted into typed coordinates (Exif 3.0 §4.6.6).
pub struct GpsInfo {
    /// Latitude, if present (paired with its N/S reference).
    pub latitude: Option<GpsCoordinate>,
    /// Longitude, if present (paired with its E/W reference).
    pub longitude: Option<GpsCoordinate>,
    /// Altitude in metres relative to sea level, if present (sign from the altitude-reference tag).
    pub altitude: Option<f64>,
}

/// A GPS coordinate: degrees/minutes/seconds as rationals, plus the hemisphere reference.
pub struct GpsCoordinate {
    /// Degrees, as an (numerator, denominator) rational.
    pub degrees: (u32, u32),
    /// Minutes, as an (numerator, denominator) rational.
    pub minutes: (u32, u32),
    /// Seconds, as an (numerator, denominator) rational.
    pub seconds: (u32, u32),
    /// The hemisphere reference for this coordinate.
    pub reference: GpsReference,
}

/// The hemisphere reference of a GPS coordinate (the `GPSLatitudeRef` / `GPSLongitudeRef` tags).
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
