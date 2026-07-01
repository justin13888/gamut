//! Profile-sequence and response-curve elements: `profileSequenceDescType` (ICC.1:2022 §10.19),
//! `profileSequenceIdentifierType` (§10.20), and `responseCurveSet16Type` (§10.21).
//!
//! These are the structurally richest §10 types: `pseq` embeds two self-delimiting description
//! elements per entry (walked with [`crate::mluc::mluc_len`]), while `psid` and `rcs2` index their
//! variable-length sub-structures through explicit offset tables relative to the element start.

use gamut_core::{Error, Result};

use crate::bytes::{ByteReader, pad_to_4, push_s15fixed16, push_xyz_number};
use crate::header::ProfileId;
use crate::mluc::{
    Mluc, TextDescription, decode_mluc, decode_text_description, encode_mluc,
    encode_text_description, mluc_len, text_description_len,
};
use crate::primitives::{S15Fixed16, Signature, XyzNumber};

// ---- profileSequenceDescType (§10.19) --------------------------------------------------------

/// A profile description embedded in `pseq`/`psid`: `multiLocalizedUnicodeType` in v4 profiles or
/// the legacy `textDescriptionType` in v2 profiles. The form is preserved so the element
/// round-trips.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescriptionText {
    /// A v4 `multiLocalizedUnicodeType` description.
    Mluc(Mluc),
    /// A v2 `textDescriptionType` description.
    TextDescription(TextDescription),
}

/// One entry of a [`ProfileSequenceDesc`] (§10.19 Table 70): the header fields and the manufacturer
/// and model descriptions of one component profile in the sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileDescription {
    /// The component profile's device manufacturer signature.
    pub device_manufacturer: Signature,
    /// The component profile's device model signature.
    pub device_model: Signature,
    /// The component profile's device attributes (header bytes 56–63).
    pub attributes: u64,
    /// The component profile's device technology signature (`0` if it had no `technologyTag`).
    pub technology: Signature,
    /// The component profile's `deviceMfgDescTag` (a placeholder empty description if it was absent).
    pub manufacturer_desc: DescriptionText,
    /// The component profile's `deviceModelDescTag`.
    pub model_desc: DescriptionText,
}

/// A `profileSequenceDescType` element (§10.19): the ordered sequence of component-profile
/// descriptions that were combined to build the profile (typically a DeviceLink).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileSequenceDesc {
    /// The description structures, in combination order (source to destination).
    pub entries: Vec<ProfileDescription>,
}

/// Decodes a `profileSequenceDescType` element.
pub(crate) fn decode_profile_sequence_desc(element: &[u8]) -> Result<ProfileSequenceDesc> {
    let mut r = ByteReader::at(element, 8)?;
    let count = r.u32()? as usize;
    let mut entries = Vec::new();
    for _ in 0..count {
        let device_manufacturer = r.signature()?;
        let device_model = r.signature()?;
        let attributes = r.u64()?;
        let technology = r.signature()?;
        let manufacturer_desc = decode_next_description(element, &mut r)?;
        let model_desc = decode_next_description(element, &mut r)?;
        entries.push(ProfileDescription {
            device_manufacturer,
            device_model,
            attributes,
            technology,
            manufacturer_desc,
            model_desc,
        });
    }
    Ok(ProfileSequenceDesc { entries })
}

/// Decodes the embedded description at the reader's current position and advances past it.
fn decode_next_description(element: &[u8], r: &mut ByteReader) -> Result<DescriptionText> {
    let rest = element
        .get(r.pos()..)
        .ok_or(Error::InvalidInput("icc: pseq truncated"))?;
    let len = description_len(rest)?;
    let slice = rest
        .get(..len)
        .ok_or(Error::InvalidInput("icc: pseq description out of bounds"))?;
    let desc = match &first_four(slice)? {
        b"mluc" => DescriptionText::Mluc(decode_mluc(slice)?),
        b"desc" => DescriptionText::TextDescription(decode_text_description(slice)?),
        _ => {
            return Err(Error::InvalidInput(
                "icc: pseq description is not mluc/desc",
            ));
        }
    };
    r.skip(len)?;
    Ok(desc)
}

/// The serialized length of the embedded description (mluc or textDescription) at `bytes`.
fn description_len(bytes: &[u8]) -> Result<usize> {
    match &first_four(bytes)? {
        b"mluc" => mluc_len(bytes),
        b"desc" => text_description_len(bytes),
        _ => Err(Error::InvalidInput(
            "icc: pseq description is not mluc/desc",
        )),
    }
}

/// The first four bytes of `bytes` as a fixed array (a type signature), erroring if too short.
fn first_four(bytes: &[u8]) -> Result<[u8; 4]> {
    let s = bytes
        .get(..4)
        .ok_or(Error::InvalidInput("icc: element too short for signature"))?;
    Ok([s[0], s[1], s[2], s[3]])
}

/// Serializes a `profileSequenceDescType` element (the inverse of [`decode_profile_sequence_desc`]).
pub(crate) fn encode_profile_sequence_desc(pseq: &ProfileSequenceDesc, out: &mut Vec<u8>) {
    out.extend_from_slice(b"pseq");
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(pseq.entries.len() as u32).to_be_bytes());
    for entry in &pseq.entries {
        out.extend_from_slice(&entry.device_manufacturer.0);
        out.extend_from_slice(&entry.device_model.0);
        out.extend_from_slice(&entry.attributes.to_be_bytes());
        out.extend_from_slice(&entry.technology.0);
        encode_description(&entry.manufacturer_desc, out);
        encode_description(&entry.model_desc, out);
    }
}

/// Serializes an embedded description in its original mluc/textDescription form.
fn encode_description(desc: &DescriptionText, out: &mut Vec<u8>) {
    match desc {
        DescriptionText::Mluc(mluc) => encode_mluc(mluc, out),
        DescriptionText::TextDescription(text) => encode_text_description(text, out),
    }
}

// ---- profileSequenceIdentifierType (§10.20) --------------------------------------------------

/// One entry of a [`ProfileSequenceIdentifier`] (§10.20 Table 72): a component profile's ID and its
/// description (always a `multiLocalizedUnicodeType`, per §10.20.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileIdentifier {
    /// The component profile's ID (its header profile ID, or a computed/zero ID).
    pub profile_id: ProfileId,
    /// The component profile's description.
    pub description: Mluc,
}

/// A `profileSequenceIdentifierType` element (§10.20): identifiers for the profiles used in a
/// sequence, indexed through a positions table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileSequenceIdentifier {
    /// The profile identifier structures, in sequence order.
    pub entries: Vec<ProfileIdentifier>,
}

/// The 16-byte profile-ID prefix of each profile-identifier structure.
const PROFILE_ID_LEN: usize = 16;

/// Decodes a `profileSequenceIdentifierType` element.
pub(crate) fn decode_profile_sequence_identifier(
    element: &[u8],
) -> Result<ProfileSequenceIdentifier> {
    let mut r = ByteReader::at(element, 8)?;
    let count = r.u32()? as usize;
    // Positions table: count × (offset, size), each a uInt32; bound it before allocating.
    if count
        .checked_mul(8)
        .and_then(|n| n.checked_add(12))
        .is_none_or(|end| end > element.len())
    {
        return Err(Error::InvalidInput(
            "icc: psid positions table exceeds element",
        ));
    }
    let mut positions = Vec::with_capacity(count);
    for _ in 0..count {
        let offset = r.u32()? as usize;
        let size = r.u32()? as usize;
        positions.push((offset, size));
    }
    let mut entries = Vec::with_capacity(count);
    for (offset, size) in positions {
        if size < PROFILE_ID_LEN {
            return Err(Error::InvalidInput(
                "icc: psid structure smaller than a profile ID",
            ));
        }
        let end = offset
            .checked_add(size)
            .ok_or(Error::InvalidInput("icc: psid structure overflow"))?;
        let structure = element
            .get(offset..end)
            .ok_or(Error::InvalidInput("icc: psid structure out of bounds"))?;
        let mut id = [0u8; PROFILE_ID_LEN];
        id.copy_from_slice(&structure[..PROFILE_ID_LEN]);
        let description = decode_mluc(&structure[PROFILE_ID_LEN..])?;
        entries.push(ProfileIdentifier {
            profile_id: ProfileId(id),
            description,
        });
    }
    Ok(ProfileSequenceIdentifier { entries })
}

/// Serializes a `profileSequenceIdentifierType` element (the inverse of
/// [`decode_profile_sequence_identifier`]). Each structure is laid out on a 4-byte boundary.
pub(crate) fn encode_profile_sequence_identifier(
    psid: &ProfileSequenceIdentifier,
    out: &mut Vec<u8>,
) {
    out.extend_from_slice(b"psid");
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(psid.entries.len() as u32).to_be_bytes());

    // The positions table (8 bytes/entry) precedes the structures; structures start 4-byte aligned.
    let structures_start = 12 + psid.entries.len() * 8;
    let mut structures = Vec::new();
    let mut table = Vec::new();
    for entry in &psid.entries {
        pad_to_4(&mut structures);
        let offset = (structures_start + structures.len()) as u32;
        let start = structures.len();
        structures.extend_from_slice(&entry.profile_id.0);
        encode_mluc(&entry.description, &mut structures);
        let size = (structures.len() - start) as u32;
        table.push((offset, size));
    }
    for (offset, size) in table {
        out.extend_from_slice(&offset.to_be_bytes());
        out.extend_from_slice(&size.to_be_bytes());
    }
    out.extend_from_slice(&structures);
}

// ---- responseCurveSet16Type (§10.21) ---------------------------------------------------------

/// One `response16Number` measurement (§10.21 Table 74): a normalized device code paired with a
/// measured colorant amount. The intervening reserved `u16` is not modelled and re-emitted as zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Response16 {
    /// The normalized device code (`0`–`65535`).
    pub device_code: u16,
    /// The measured value for that device code, as an `s15Fixed16`.
    pub measurement: S15Fixed16,
}

/// One response curve of a [`ResponseCurveSet16`] (§10.21 Table 74): the reference response of a
/// device in one measurement unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseCurve {
    /// The measurement-unit signature (§10.21 Table 75, e.g. `StaA`, `StaT`, `DN  `).
    pub measurement_unit: Signature,
    /// The maximum-colorant PCSXYZ measurement of each channel (one per channel).
    pub pcs_values: Vec<XyzNumber>,
    /// The response array of each channel, ordered by channel.
    pub responses: Vec<Vec<Response16>>,
}

/// A `responseCurveSet16Type` element (§10.21): per-channel reference responses in one or more
/// measurement units, used to compensate for device drift without re-profiling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResponseCurveSet16 {
    /// The number of device channels shared by every curve.
    pub channels: u16,
    /// The response curves, one per measurement type.
    pub curves: Vec<ResponseCurve>,
}

/// Decodes a `responseCurveSet16Type` element.
pub(crate) fn decode_response_curve_set16(element: &[u8]) -> Result<ResponseCurveSet16> {
    let mut r = ByteReader::at(element, 8)?;
    let channels = r.u16()?;
    let measurement_count = r.u16()? as usize;
    let n = channels as usize;
    let mut offsets = Vec::with_capacity(measurement_count);
    for _ in 0..measurement_count {
        offsets.push(r.u32()? as usize);
    }
    let mut curves = Vec::with_capacity(measurement_count);
    for offset in offsets {
        curves.push(decode_response_curve(element, offset, n)?);
    }
    Ok(ResponseCurveSet16 { channels, curves })
}

/// Decodes one response curve structure at `offset` for `n` channels.
fn decode_response_curve(element: &[u8], offset: usize, n: usize) -> Result<ResponseCurve> {
    let mut r = ByteReader::at(element, offset)?;
    let measurement_unit = r.signature()?;
    let mut counts = Vec::with_capacity(n);
    for _ in 0..n {
        counts.push(r.u32()? as usize);
    }
    let mut pcs_values = Vec::with_capacity(n);
    for _ in 0..n {
        pcs_values.push(r.xyz_number()?);
    }
    let mut responses = Vec::with_capacity(n);
    for &count in &counts {
        // Each response16Number is 8 bytes; bound the array against the element before allocating.
        if count
            .checked_mul(8)
            .is_none_or(|bytes| bytes > r.remaining())
        {
            return Err(Error::InvalidInput(
                "icc: rcs2 response array exceeds element",
            ));
        }
        let mut array = Vec::with_capacity(count);
        for _ in 0..count {
            let device_code = r.u16()?;
            r.skip(2)?; // the reserved u16
            let measurement = r.s15fixed16()?;
            array.push(Response16 {
                device_code,
                measurement,
            });
        }
        responses.push(array);
    }
    Ok(ResponseCurve {
        measurement_unit,
        pcs_values,
        responses,
    })
}

/// Serializes a `responseCurveSet16Type` element (the inverse of [`decode_response_curve_set16`]).
pub(crate) fn encode_response_curve_set16(rcs: &ResponseCurveSet16, out: &mut Vec<u8>) {
    out.extend_from_slice(b"rcs2");
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&rcs.channels.to_be_bytes());
    out.extend_from_slice(&(rcs.curves.len() as u16).to_be_bytes());

    let structures_start = 12 + rcs.curves.len() * 4;
    let mut structures = Vec::new();
    let mut offsets = Vec::new();
    for curve in &rcs.curves {
        offsets.push((structures_start + structures.len()) as u32);
        encode_response_curve(curve, &mut structures);
    }
    for offset in offsets {
        out.extend_from_slice(&offset.to_be_bytes());
    }
    out.extend_from_slice(&structures);
}

/// Serializes one response curve structure.
fn encode_response_curve(curve: &ResponseCurve, out: &mut Vec<u8>) {
    out.extend_from_slice(&curve.measurement_unit.0);
    for array in &curve.responses {
        out.extend_from_slice(&(array.len() as u32).to_be_bytes());
    }
    for xyz in &curve.pcs_values {
        push_xyz_number(out, *xyz);
    }
    for array in &curve.responses {
        for response in array {
            out.extend_from_slice(&response.device_code.to_be_bytes());
            out.extend_from_slice(&[0, 0]); // reserved
            push_s15fixed16(out, response.measurement);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mluc::MlucRecord;

    fn en_us(text: &str) -> Mluc {
        Mluc {
            records: vec![MlucRecord {
                language: *b"en",
                country: *b"US",
                text: text.to_owned(),
            }],
        }
    }

    fn sample_pseq() -> ProfileSequenceDesc {
        ProfileSequenceDesc {
            entries: vec![
                ProfileDescription {
                    device_manufacturer: Signature(*b"APPL"),
                    device_model: Signature(*b"mdl1"),
                    attributes: 0,
                    technology: Signature(*b"CRT "),
                    // A v4 (mluc) manufacturer description and a v2 (desc) model description, so the
                    // round-trip exercises both embedded forms and the length walker.
                    manufacturer_desc: DescriptionText::Mluc(en_us("Widgets Inc")),
                    model_desc: DescriptionText::TextDescription(TextDescription {
                        ascii: "Model One".to_owned(),
                        ..TextDescription::default()
                    }),
                },
                ProfileDescription {
                    device_manufacturer: Signature::ZERO,
                    device_model: Signature::ZERO,
                    attributes: 0x0000_0001_0000_0002,
                    technology: Signature::ZERO,
                    manufacturer_desc: DescriptionText::Mluc(Mluc::default()), // placeholder
                    model_desc: DescriptionText::Mluc(en_us("Final")),
                },
            ],
        }
    }

    #[test]
    fn profile_sequence_desc_round_trips_through_encode() {
        let pseq = sample_pseq();
        let mut out = Vec::new();
        encode_profile_sequence_desc(&pseq, &mut out);
        assert_eq!(decode_profile_sequence_desc(&out).unwrap(), pseq);
    }

    #[test]
    fn profile_sequence_desc_count_and_fields_decode() {
        let pseq = sample_pseq();
        let mut out = Vec::new();
        encode_profile_sequence_desc(&pseq, &mut out);
        let decoded = decode_profile_sequence_desc(&out).unwrap();
        assert_eq!(decoded.entries.len(), 2);
        assert_eq!(decoded.entries[0].device_manufacturer, Signature(*b"APPL"));
        assert_eq!(decoded.entries[1].attributes, 0x0000_0001_0000_0002);
    }

    #[test]
    fn profile_sequence_identifier_round_trips_through_encode() {
        let psid = ProfileSequenceIdentifier {
            entries: vec![
                ProfileIdentifier {
                    profile_id: ProfileId([1; 16]),
                    description: en_us("First"),
                },
                ProfileIdentifier {
                    profile_id: ProfileId([2; 16]),
                    description: en_us("Second"),
                },
            ],
        };
        let mut out = Vec::new();
        encode_profile_sequence_identifier(&psid, &mut out);
        assert_eq!(decode_profile_sequence_identifier(&out).unwrap(), psid);
    }

    #[test]
    fn response_curve_set16_round_trips_through_encode() {
        let rcs = ResponseCurveSet16 {
            channels: 2,
            curves: vec![ResponseCurve {
                measurement_unit: Signature(*b"StaT"),
                pcs_values: vec![
                    XyzNumber::from_f64([0.9, 1.0, 0.8]),
                    XyzNumber::from_f64([0.1, 0.1, 0.1]),
                ],
                responses: vec![
                    vec![
                        Response16 {
                            device_code: 0,
                            measurement: S15Fixed16::from_f64(0.0),
                        },
                        Response16 {
                            device_code: 65535,
                            measurement: S15Fixed16::from_f64(2.5),
                        },
                    ],
                    vec![Response16 {
                        device_code: 32768,
                        measurement: S15Fixed16::from_f64(1.25),
                    }],
                ],
            }],
        };
        let mut out = Vec::new();
        encode_response_curve_set16(&rcs, &mut out);
        assert_eq!(decode_response_curve_set16(&out).unwrap(), rcs);
    }

    #[test]
    fn response_curve_set16_offsets_locate_each_curve() {
        // Two measurement types over one channel: the offset table must direct each decode.
        let rcs = ResponseCurveSet16 {
            channels: 1,
            curves: vec![
                ResponseCurve {
                    measurement_unit: Signature(*b"StaA"),
                    pcs_values: vec![XyzNumber::from_f64([0.5, 0.5, 0.5])],
                    responses: vec![vec![Response16 {
                        device_code: 100,
                        measurement: S15Fixed16::from_f64(0.5),
                    }]],
                },
                ResponseCurve {
                    measurement_unit: Signature(*b"StaE"),
                    pcs_values: vec![XyzNumber::from_f64([0.6, 0.6, 0.6])],
                    responses: vec![vec![Response16 {
                        device_code: 200,
                        measurement: S15Fixed16::from_f64(0.75),
                    }]],
                },
            ],
        };
        let mut out = Vec::new();
        encode_response_curve_set16(&rcs, &mut out);
        let decoded = decode_response_curve_set16(&out).unwrap();
        assert_eq!(decoded.curves.len(), 2);
        assert_eq!(decoded.curves[0].measurement_unit, Signature(*b"StaA"));
        assert_eq!(decoded.curves[1].measurement_unit, Signature(*b"StaE"));
    }
}
