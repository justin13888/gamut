//! Colour-measurement and viewing-condition element types: `chromaticityType` (ICC.1:2022 §10.2),
//! `measurementType` (§10.14), and `viewingConditionsType` (§10.30).
//!
//! Each carries measurement metadata rather than a transform. The observer/geometry/illuminant
//! selectors are kept as their raw `u32` code points (not enums) so a profile carrying a reserved
//! or future code still decodes and round-trips losslessly; §10.14 Tables 50/51/53 give the defined
//! values.

use crate::bytes::{ByteReader, push_u16fixed16, push_xyz_number};
use crate::error::{IccError, Result};
use crate::primitives::{U16Fixed16, XyzNumber};

/// A `chromaticityType` element (§10.2): the CIE `xy` chromaticities of a display's phosphors or a
/// device's colorants, one `(x, y)` pair per device channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chromaticity {
    /// The encoded phosphor/colorant type (§10.2 Table 31): `0` = unknown (chromaticities given
    /// explicitly); `1`–`6` name a standard primary set (ITU-R BT.709-2, SMPTE RP145,
    /// EBU Tech.3213-E, P22, P3, ITU-R BT.2020), for which the channel count is three.
    pub colorant_type: u16,
    /// The CIE `[x, y]` chromaticity of each device channel, as `u16Fixed16` pairs.
    pub channels: Vec<[U16Fixed16; 2]>,
}

/// Decodes a `chromaticityType` element.
pub(crate) fn decode_chromaticity(element: &[u8]) -> Result<Chromaticity> {
    let mut r = ByteReader::at(element, 8)?;
    let channel_count = r.u16()? as usize;
    let colorant_type = r.u16()?;
    // Bound the channel data (8 bytes per channel) against the element before allocating.
    if channel_count * 8 > r.remaining() {
        return Err(IccError::Malformed(
            "icc: chromaticity channels exceed element",
        ));
    }
    let mut channels = Vec::with_capacity(channel_count);
    for _ in 0..channel_count {
        channels.push([r.u16fixed16()?, r.u16fixed16()?]);
    }
    Ok(Chromaticity {
        colorant_type,
        channels,
    })
}

/// Serializes a `chromaticityType` element (the inverse of [`decode_chromaticity`]).
pub(crate) fn encode_chromaticity(chrm: &Chromaticity, out: &mut Vec<u8>) {
    out.extend_from_slice(b"chrm");
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(chrm.channels.len() as u16).to_be_bytes());
    out.extend_from_slice(&chrm.colorant_type.to_be_bytes());
    for [x, y] in &chrm.channels {
        push_u16fixed16(out, *x);
        push_u16fixed16(out, *y);
    }
}

/// A `measurementType` element (§10.14): the measurement conditions used to characterize the
/// profile, offered as an alternative to the default measurement specification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Measurement {
    /// Standard observer code (§10.14 Table 50): `0` unknown, `1` CIE 1931, `2` CIE 1964.
    pub observer: u32,
    /// The nCIEXYZ tristimulus of the measurement backing.
    pub backing: XyzNumber,
    /// Measurement geometry code (Table 51): `0` unknown, `1` 0°:45°/45°:0°, `2` 0°:d/d:0°.
    pub geometry: u32,
    /// Measurement flare (Table 52), `0.0`–`1.0` as a `u16Fixed16`.
    pub flare: U16Fixed16,
    /// Standard illuminant code (Table 53): `0` unknown, `1` D50, `2` D65, `3` D93, `4` F2, `5` D55,
    /// `6` A, `7` Equi-Power (E), `8` F8.
    pub illuminant: u32,
}

/// Decodes a `measurementType` element.
pub(crate) fn decode_measurement(element: &[u8]) -> Result<Measurement> {
    let mut r = ByteReader::at(element, 8)?;
    Ok(Measurement {
        observer: r.u32()?,
        backing: r.xyz_number()?,
        geometry: r.u32()?,
        flare: r.u16fixed16()?,
        illuminant: r.u32()?,
    })
}

/// Serializes a `measurementType` element (the inverse of [`decode_measurement`]).
pub(crate) fn encode_measurement(meas: &Measurement, out: &mut Vec<u8>) {
    out.extend_from_slice(b"meas");
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&meas.observer.to_be_bytes());
    push_xyz_number(out, meas.backing);
    out.extend_from_slice(&meas.geometry.to_be_bytes());
    push_u16fixed16(out, meas.flare);
    out.extend_from_slice(&meas.illuminant.to_be_bytes());
}

/// A `viewingConditionsType` element (§10.30): the viewing conditions the media is defined for, in
/// un-normalized CIEXYZ (the illuminant/surround `Y` are luminances in cd/m²).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewingConditions {
    /// Un-normalized CIEXYZ of the illuminant (`Y` in cd/m²).
    pub illuminant: XyzNumber,
    /// Un-normalized CIEXYZ of the surround (`Y` in cd/m²).
    pub surround: XyzNumber,
    /// Illuminant type code, encoded as in [`Measurement::illuminant`] (§10.14 Table 53).
    pub illuminant_type: u32,
}

/// Decodes a `viewingConditionsType` element.
pub(crate) fn decode_viewing_conditions(element: &[u8]) -> Result<ViewingConditions> {
    let mut r = ByteReader::at(element, 8)?;
    Ok(ViewingConditions {
        illuminant: r.xyz_number()?,
        surround: r.xyz_number()?,
        illuminant_type: r.u32()?,
    })
}

/// Serializes a `viewingConditionsType` element (the inverse of [`decode_viewing_conditions`]).
pub(crate) fn encode_viewing_conditions(view: &ViewingConditions, out: &mut Vec<u8>) {
    out.extend_from_slice(b"view");
    out.extend_from_slice(&[0; 4]);
    push_xyz_number(out, view.illuminant);
    push_xyz_number(out, view.surround);
    out.extend_from_slice(&view.illuminant_type.to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::IccError;
    use crate::primitives::S15Fixed16;

    /// Builds an element: a four-byte type signature, four reserved bytes, then the payload.
    fn element(type_sig: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut e = Vec::with_capacity(8 + payload.len());
        e.extend_from_slice(type_sig);
        e.extend_from_slice(&[0; 4]);
        e.extend_from_slice(payload);
        e
    }

    #[test]
    fn chromaticity_decode_bounds_are_exact() {
        // A single channel exactly filling the element must decode: the bound is channels × 8
        // bytes, so any arithmetic drift (e.g. `1 + 8 = 9`) would falsely reject this fit.
        let mut payload = Vec::new();
        payload.extend_from_slice(&1u16.to_be_bytes()); // channel count
        payload.extend_from_slice(&0u16.to_be_bytes()); // colorant type
        payload.extend_from_slice(&U16Fixed16::from_f64(0.64).0.to_be_bytes());
        payload.extend_from_slice(&U16Fixed16::from_f64(0.33).0.to_be_bytes());
        let chrm = decode_chromaticity(&element(b"chrm", &payload)).unwrap();
        assert_eq!(chrm.channels.len(), 1);

        // Trailing slack is tolerated (the guard rejects only data that exceeds the element).
        let mut padded = payload.clone();
        padded.extend_from_slice(&[0xAB; 8]);
        assert!(decode_chromaticity(&element(b"chrm", &padded)).is_ok());

        // A truncated element is caught by the size guard itself (channels × 8, not fewer), so
        // the guard's message is reported, not a later read error.
        let mut truncated = Vec::new();
        truncated.extend_from_slice(&3u16.to_be_bytes());
        truncated.extend_from_slice(&0u16.to_be_bytes());
        truncated.extend_from_slice(&[0u8; 20]); // 20 < 3 × 8
        match decode_chromaticity(&element(b"chrm", &truncated)) {
            Err(IccError::Malformed(msg)) => {
                assert_eq!(msg, "icc: chromaticity channels exceed element");
            }
            other => panic!("expected the size-guard error, got {other:?}"),
        }
    }

    #[test]
    fn decodes_chromaticity_channels() {
        // Two channels, colorant type 0 (explicit), xy = (0.64, 0.33) and (0.30, 0.60).
        let xy = [0.64_f64, 0.33, 0.30, 0.60].map(U16Fixed16::from_f64);
        let mut payload = 2u16.to_be_bytes().to_vec(); // channel count
        payload.extend_from_slice(&0u16.to_be_bytes()); // colorant type
        for v in xy {
            payload.extend_from_slice(&v.0.to_be_bytes());
        }
        let chrm = decode_chromaticity(&element(b"chrm", &payload)).unwrap();
        assert_eq!(chrm.colorant_type, 0);
        assert_eq!(chrm.channels.len(), 2);
        assert_eq!(chrm.channels[0], [xy[0], xy[1]]);
        assert_eq!(chrm.channels[1], [xy[2], xy[3]]);
    }

    #[test]
    fn rejects_chromaticity_with_truncated_channels() {
        // Claims three channels but carries data for none.
        let mut payload = 3u16.to_be_bytes().to_vec();
        payload.extend_from_slice(&1u16.to_be_bytes());
        assert!(decode_chromaticity(&element(b"chrm", &payload)).is_err());
    }

    #[test]
    fn measurement_round_trips_through_encode() {
        let meas = Measurement {
            observer: 1,
            backing: XyzNumber {
                x: S15Fixed16(0),
                y: S15Fixed16(0),
                z: S15Fixed16(0),
            },
            geometry: 2,
            flare: U16Fixed16::from_f64(0.01),
            illuminant: 1, // D50
        };
        let mut out = Vec::new();
        encode_measurement(&meas, &mut out);
        assert_eq!(decode_measurement(&out).unwrap(), meas);
    }

    #[test]
    fn measurement_decodes_each_field_at_its_offset() {
        // observer=2, backing=D50, geometry=1, flare=1.0, illuminant=8 (F8).
        let mut payload = 2u32.to_be_bytes().to_vec();
        for raw in [0x0000_F6D6_i32, 0x0001_0000, 0x0000_D32D] {
            payload.extend_from_slice(&raw.to_be_bytes());
        }
        payload.extend_from_slice(&1u32.to_be_bytes());
        payload.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // flare 1.0
        payload.extend_from_slice(&8u32.to_be_bytes());
        let meas = decode_measurement(&element(b"meas", &payload)).unwrap();
        assert_eq!(meas.observer, 2);
        assert_eq!(meas.backing.x, S15Fixed16(0x0000_F6D6));
        assert_eq!(meas.geometry, 1);
        assert_eq!(meas.flare.to_f64(), 1.0);
        assert_eq!(meas.illuminant, 8);
    }

    #[test]
    fn viewing_conditions_round_trips_through_encode() {
        let view = ViewingConditions {
            illuminant: XyzNumber::from_f64([19.0, 20.0, 21.0]),
            surround: XyzNumber::from_f64([0.4, 0.4, 0.4]),
            illuminant_type: 1,
        };
        let mut out = Vec::new();
        encode_viewing_conditions(&view, &mut out);
        assert_eq!(decode_viewing_conditions(&out).unwrap(), view);
    }
}
