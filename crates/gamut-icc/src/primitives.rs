//! ICC binary numeric primitives (ICC.1:2022 §4).
//!
//! The fixed-point encodings and small composite number types an ICC profile is built from. Each is
//! a newtype over its **raw** on-disk integer, so a parsed value round-trips to byte-identical
//! output, with `to_f64`/`from_f64` accessors for ergonomic use. The fixed-point conversions are
//! exact integer scalings:
//!
//! * `s15Fixed16` and `u16Fixed16` — value = `raw / 65536`;
//! * `u8Fixed8` — value = `raw / 256`.

/// An `s15Fixed16Number` (ICC.1:2022 §4.6): a signed 16.16 fixed-point value stored big-endian as
/// an `i32`. The numeric value is `raw / 65536` (e.g. `0x0001_0000` = 1.0, `0xFFFF_0000` = -1.0).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct S15Fixed16(pub i32);

impl S15Fixed16 {
    /// The value as `f64` (`raw / 65536`). Exact — every `i32` is representable in `f64`.
    #[must_use]
    pub fn to_f64(self) -> f64 {
        f64::from(self.0) / 65536.0
    }

    /// The nearest `s15Fixed16` to `v` (round half away from zero), saturating values outside the
    /// representable range.
    #[must_use]
    pub fn from_f64(v: f64) -> Self {
        let scaled = (v * 65536.0).round();
        Self(scaled.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32)
    }
}

/// A `u16Fixed16Number` (ICC.1:2022 §4.7): an unsigned 16.16 fixed-point value stored big-endian as
/// a `u32`. The numeric value is `raw / 65536`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct U16Fixed16(pub u32);

impl U16Fixed16 {
    /// The value as `f64` (`raw / 65536`). Exact — every `u32` is representable in `f64`.
    #[must_use]
    pub fn to_f64(self) -> f64 {
        f64::from(self.0) / 65536.0
    }

    /// The nearest `u16Fixed16` to `v` (round half away from zero), saturating to `[0, u32::MAX]`.
    #[must_use]
    pub fn from_f64(v: f64) -> Self {
        let scaled = (v * 65536.0).round();
        Self(scaled.clamp(0.0, f64::from(u32::MAX)) as u32)
    }
}

/// A `u8Fixed8Number` (ICC.1:2022 §4.5): an unsigned 8.8 fixed-point value stored big-endian as a
/// `u16`. The numeric value is `raw / 256`. Used by the single-entry `curveType` gamma encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct U8Fixed8(pub u16);

impl U8Fixed8 {
    /// The value as `f64` (`raw / 256`). Exact.
    #[must_use]
    pub fn to_f64(self) -> f64 {
        f64::from(self.0) / 256.0
    }

    /// The nearest `u8Fixed8` to `v` (round half away from zero), saturating to `[0, u16::MAX]`.
    #[must_use]
    pub fn from_f64(v: f64) -> Self {
        let scaled = (v * 256.0).round();
        Self(scaled.clamp(0.0, f64::from(u16::MAX)) as u16)
    }
}

/// An `XYZNumber` (ICC.1:2022 §4.14): three `s15Fixed16` components. The header PCS illuminant and
/// every `XYZType` value use this representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct XyzNumber {
    /// The X tristimulus component.
    pub x: S15Fixed16,
    /// The Y tristimulus component.
    pub y: S15Fixed16,
    /// The Z tristimulus component.
    pub z: S15Fixed16,
}

impl XyzNumber {
    /// `[X, Y, Z]` as `f64` — the bridge to floating-point colour math.
    #[must_use]
    pub fn to_f64(self) -> [f64; 3] {
        [self.x.to_f64(), self.y.to_f64(), self.z.to_f64()]
    }

    /// The nearest `XYZNumber` to `[X, Y, Z]`, each component rounded and saturated per
    /// [`S15Fixed16::from_f64`].
    #[must_use]
    pub fn from_f64(xyz: [f64; 3]) -> Self {
        Self {
            x: S15Fixed16::from_f64(xyz[0]),
            y: S15Fixed16::from_f64(xyz[1]),
            z: S15Fixed16::from_f64(xyz[2]),
        }
    }
}

/// A `dateTimeNumber` (ICC.1:2022 §4.2): a UTC calendar timestamp as six `u16` fields.
///
/// The components are stored verbatim and never validated or normalized: ICC permits an all-zero
/// "unset" timestamp (see [`DateTime::is_zero`]), and out-of-range values appear in real profiles,
/// so preserving the raw fields is what keeps a parse → serialize round-trip lossless.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DateTime {
    /// Year (e.g. 2026).
    pub year: u16,
    /// Month, 1–12.
    pub month: u16,
    /// Day of month, 1–31.
    pub day: u16,
    /// Hours, 0–23.
    pub hours: u16,
    /// Minutes, 0–59.
    pub minutes: u16,
    /// Seconds, 0–59.
    pub seconds: u16,
}

impl DateTime {
    /// The all-zero timestamp, which ICC profiles use to mean "unset".
    pub const ZERO: DateTime = DateTime {
        year: 0,
        month: 0,
        day: 0,
        hours: 0,
        minutes: 0,
        seconds: 0,
    };

    /// Whether every field is zero (the "unset" timestamp).
    #[must_use]
    pub fn is_zero(self) -> bool {
        self == Self::ZERO
    }
}

/// A four-byte signature (ICC.1:2022 §4.12): a four-character code stored big-endian.
///
/// Used for the open-registry header fields — preferred CMM, primary platform, device
/// manufacturer/model, profile creator — where the all-zero value means "unspecified", and for the
/// `signatureType` tag element.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Signature(pub [u8; 4]);

impl Signature {
    /// The all-zero signature, meaning "unspecified".
    pub const ZERO: Signature = Signature([0; 4]);

    /// The signature's four bytes as a big-endian `u32`.
    #[must_use]
    pub fn to_u32(self) -> u32 {
        u32::from_be_bytes(self.0)
    }

    /// A signature from a big-endian `u32`.
    #[must_use]
    pub fn from_u32(value: u32) -> Self {
        Self(value.to_be_bytes())
    }
}

impl From<[u8; 4]> for Signature {
    /// Wraps four bytes as a signature — `Signature::from(*b"wtpt")`.
    fn from(bytes: [u8; 4]) -> Self {
        Self(bytes)
    }
}

impl core::fmt::Display for Signature {
    /// Renders printable bytes verbatim (e.g. `RGB `) and any others as `\xNN`.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for &b in &self.0 {
            if b.is_ascii_graphic() || b == b' ' {
                write!(f, "{}", b as char)?;
            } else {
                write!(f, "\\x{b:02x}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn s15fixed16_to_f64_known_vectors() {
        // (raw, value): the spec's reference points plus the D50 components.
        for (raw, expected) in [
            (0x0001_0000_i32, 1.0_f64),
            (0xFFFF_0000_u32 as i32, -1.0),
            (0x0000_8000, 0.5),
            (0, 0.0),
            (0x0000_0001, 1.0 / 65536.0),
        ] {
            assert_eq!(S15Fixed16(raw).to_f64(), expected, "raw={raw:#010x}");
        }
    }

    #[test]
    fn s15fixed16_from_f64_rounds_and_saturates() {
        // Round half away from zero: 2/3 * 65536 = 43690.66 → 43691 (truncation would give 43690).
        assert_eq!(S15Fixed16::from_f64(2.0 / 3.0), S15Fixed16(43691));
        assert_eq!(S15Fixed16::from_f64(1.0), S15Fixed16(0x0001_0000));
        assert_eq!(
            S15Fixed16::from_f64(-1.0),
            S15Fixed16(0xFFFF_0000_u32 as i32)
        );
        // Out of range saturates rather than wrapping.
        assert_eq!(S15Fixed16::from_f64(1e9), S15Fixed16(i32::MAX));
        assert_eq!(S15Fixed16::from_f64(-1e9), S15Fixed16(i32::MIN));
    }

    #[test]
    fn u16fixed16_round_trip() {
        assert_eq!(U16Fixed16(0x0002_0000).to_f64(), 2.0);
        assert_eq!(U16Fixed16::from_f64(2.0), U16Fixed16(0x0002_0000));
        assert_eq!(U16Fixed16::from_f64(-1.0), U16Fixed16(0)); // clamps at zero
    }

    #[test]
    fn u8fixed8_known_vectors() {
        assert_eq!(U8Fixed8(0x0100).to_f64(), 1.0);
        assert_eq!(U8Fixed8(0x0180).to_f64(), 1.5);
        assert_eq!(
            U8Fixed8::from_f64(2.2),
            U8Fixed8((2.2 * 256.0_f64).round() as u16)
        );
    }

    #[test]
    fn xyz_number_d50_round_trip() {
        let d50 = [0.9642, 1.0, 0.8249];
        let round_tripped = XyzNumber::from_f64(d50).to_f64();
        for (got, want) in round_tripped.iter().zip(d50.iter()) {
            assert!((got - want).abs() < 2.0 / 65536.0, "{got} vs {want}");
        }
    }

    #[test]
    fn datetime_zero_is_unset() {
        assert!(DateTime::ZERO.is_zero());
        assert!(
            !DateTime {
                year: 2026,
                ..DateTime::ZERO
            }
            .is_zero()
        );
    }

    #[test]
    fn signature_u32_round_trip_and_display() {
        let sig = Signature(*b"RGB ");
        assert_eq!(sig.to_u32(), 0x5247_4220);
        assert_eq!(Signature::from_u32(0x5247_4220), sig);
        assert_eq!(sig.to_string(), "RGB ");
        // Non-printable bytes render as escapes.
        assert_eq!(
            Signature([0, 1, b'A', 0xff]).to_string(),
            "\\x00\\x01A\\xff"
        );
    }
}
