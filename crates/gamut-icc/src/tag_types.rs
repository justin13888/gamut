//! The decoded element data a tag can hold (ICC.1:2022 §10).

use gamut_core::{Error, Result};

use crate::bytes::{ByteReader, push_date_time, push_s15fixed16, push_xyz_number};
use crate::curve::{
    Curve, ParametricCurve, read_curve_body, read_parametric_body, write_curve_body,
    write_parametric_body,
};
use crate::lut::{
    Lut8, Lut16, LutAToB, LutBToA, decode_lut_a_to_b, decode_lut_b_to_a, decode_lut8, decode_lut16,
    encode_lut_a_to_b, encode_lut_b_to_a, encode_lut8, encode_lut16,
};
use crate::mluc::{
    Mluc, TextDescription, decode_mluc, decode_text_description, encode_mluc,
    encode_text_description,
};
use crate::named_color::{NamedColor2, decode_named_color2, encode_named_color2};
use crate::primitives::{DateTime, S15Fixed16, Signature, XyzNumber};

/// The decoded data of a tag element.
///
/// Each variant models one ICC element type. [`TagData::Raw`] carries any element type gamut-icc
/// does not decode semantically — verbatim — so every tag round-trips byte-for-byte regardless of
/// whether it is modelled. The enum is `#[non_exhaustive]`: variants are added as more element
/// types gain semantic decoders.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum TagData {
    /// `XYZ ` — one or more CIE XYZ triplets (`XYZType`); colorant, white/black point, luminance.
    Xyz(Vec<XyzNumber>),
    /// `curv` — a sampled/identity/gamma tone curve (`curveType`).
    Curve(Curve),
    /// `para` — a parametric tone curve (`parametricCurveType`).
    ParametricCurve(ParametricCurve),
    /// `text` — 7-bit ASCII text (`textType`), NUL terminator stripped.
    Text(String),
    /// `dtim` — a date-time (`dateTimeType`).
    DateTime(DateTime),
    /// `sig ` — a four-byte signature value (`signatureType`).
    Signature(Signature),
    /// `sf32` — an array of `s15Fixed16` numbers (`s15Fixed16ArrayType`, e.g. `chad`).
    S15Fixed16Array(Vec<S15Fixed16>),
    /// `mluc` — language-tagged Unicode text (`multiLocalizedUnicodeType`), the v4 form of `desc`.
    MultiLocalizedUnicode(Mluc),
    /// `desc` — the legacy v2 description element (`textDescriptionType`).
    TextDescription(TextDescription),
    /// `mft1` — the legacy 8-bit lookup transform (`lut8Type`).
    Lut8(Lut8),
    /// `mft2` — the legacy 16-bit lookup transform (`lut16Type`).
    Lut16(Lut16),
    /// `mAB ` — the device-to-PCS lookup transform (`lutAToBType`).
    LutAToB(LutAToB),
    /// `mBA ` — the PCS-to-device lookup transform (`lutBToAType`).
    LutBToA(LutBToA),
    /// `ncl2` — a named-colour palette (`namedColor2Type`).
    NamedColor2(NamedColor2),
    /// An element gamut-icc does not model semantically: the complete element bytes verbatim,
    /// including the leading four-byte type signature and its four reserved bytes. Re-emitted
    /// exactly on serialization.
    Raw {
        /// The element's four-byte type signature (the first four bytes of `bytes`).
        type_sig: Signature,
        /// The complete element bytes.
        bytes: Vec<u8>,
    },
}

/// Decodes one tag element from its bytes; the element begins with its four-byte type signature
/// followed by four reserved bytes (ICC.1:2022 §10).
///
/// Modelled element types are decoded semantically; any other type (or an element too short to carry
/// a type signature) is preserved as [`TagData::Raw`], which round-trips verbatim. A *modelled* type
/// whose payload is malformed is a parse error rather than a silent fallback.
pub(crate) fn decode_tag(element: &[u8]) -> Result<TagData> {
    let type_sig = element_type_signature(element);
    match &type_sig.0 {
        b"XYZ " => decode_xyz(element),
        b"curv" => Ok(TagData::Curve(decode_curve(element)?)),
        b"para" => Ok(TagData::ParametricCurve(decode_parametric(element)?)),
        b"text" => decode_text(element),
        b"dtim" => Ok(TagData::DateTime(decode_date_time(element)?)),
        b"sig " => Ok(TagData::Signature(decode_signature(element)?)),
        b"sf32" => decode_s15fixed16_array(element),
        b"mluc" => Ok(TagData::MultiLocalizedUnicode(decode_mluc(element)?)),
        b"desc" => Ok(TagData::TextDescription(decode_text_description(element)?)),
        b"mft1" => Ok(TagData::Lut8(decode_lut8(element)?)),
        b"mft2" => Ok(TagData::Lut16(decode_lut16(element)?)),
        b"mAB " => Ok(TagData::LutAToB(decode_lut_a_to_b(element)?)),
        b"mBA " => Ok(TagData::LutBToA(decode_lut_b_to_a(element)?)),
        b"ncl2" => Ok(TagData::NamedColor2(decode_named_color2(element)?)),
        _ => Ok(TagData::Raw {
            type_sig,
            bytes: element.to_vec(),
        }),
    }
}

/// The element's four-byte type signature, or [`Signature::ZERO`] for an element shorter than four
/// bytes (a malformed element the caller still round-trips verbatim).
fn element_type_signature(element: &[u8]) -> Signature {
    match element.get(..4) {
        Some(s) => Signature([s[0], s[1], s[2], s[3]]),
        None => Signature::ZERO,
    }
}

fn decode_xyz(element: &[u8]) -> Result<TagData> {
    let mut r = ByteReader::at(element, 8)?;
    let count = r.remaining() / 12;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(r.xyz_number()?);
    }
    Ok(TagData::Xyz(values))
}

fn decode_curve(element: &[u8]) -> Result<Curve> {
    read_curve_body(&mut ByteReader::at(element, 8)?)
}

fn decode_parametric(element: &[u8]) -> Result<ParametricCurve> {
    read_parametric_body(&mut ByteReader::at(element, 8)?)
}

fn decode_text(element: &[u8]) -> Result<TagData> {
    let mut r = ByteReader::at(element, 8)?;
    let n = r.remaining();
    let raw = r.bytes(n)?;
    if !raw.is_ascii() {
        return Err(Error::InvalidInput("icc: non-ASCII textType"));
    }
    let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    let text: String = raw[..end].iter().map(|&b| b as char).collect();
    Ok(TagData::Text(text))
}

fn decode_date_time(element: &[u8]) -> Result<DateTime> {
    ByteReader::at(element, 8)?.date_time()
}

fn decode_signature(element: &[u8]) -> Result<Signature> {
    ByteReader::at(element, 8)?.signature()
}

fn decode_s15fixed16_array(element: &[u8]) -> Result<TagData> {
    let mut r = ByteReader::at(element, 8)?;
    let count = r.remaining() / 4;
    let mut values = Vec::with_capacity(count);
    for _ in 0..count {
        values.push(r.s15fixed16()?);
    }
    Ok(TagData::S15Fixed16Array(values))
}

/// Serializes a tag element (its four-byte type signature, four reserved bytes, then payload) into
/// `out` — the inverse of [`decode_tag`]. [`TagData::Raw`] re-emits its stored bytes verbatim.
pub(crate) fn encode_tag(data: &TagData, out: &mut Vec<u8>) {
    match data {
        TagData::Xyz(values) => {
            element_header(out, b"XYZ ");
            for &value in values {
                push_xyz_number(out, value);
            }
        }
        TagData::Curve(curve) => {
            element_header(out, b"curv");
            write_curve_body(curve, out);
        }
        TagData::ParametricCurve(curve) => {
            element_header(out, b"para");
            write_parametric_body(curve, out);
        }
        TagData::Text(text) => {
            element_header(out, b"text");
            out.extend_from_slice(text.as_bytes());
            out.push(0); // NUL terminator
        }
        TagData::DateTime(date_time) => {
            element_header(out, b"dtim");
            push_date_time(out, *date_time);
        }
        TagData::Signature(signature) => {
            element_header(out, b"sig ");
            out.extend_from_slice(&signature.0);
        }
        TagData::S15Fixed16Array(values) => {
            element_header(out, b"sf32");
            for &value in values {
                push_s15fixed16(out, value);
            }
        }
        TagData::MultiLocalizedUnicode(mluc) => encode_mluc(mluc, out),
        TagData::TextDescription(desc) => encode_text_description(desc, out),
        TagData::Lut8(lut) => encode_lut8(lut, out),
        TagData::Lut16(lut) => encode_lut16(lut, out),
        TagData::LutAToB(lut) => encode_lut_a_to_b(lut, out),
        TagData::LutBToA(lut) => encode_lut_b_to_a(lut, out),
        TagData::NamedColor2(named) => encode_named_color2(named, out),
        TagData::Raw { bytes, .. } => out.extend_from_slice(bytes),
    }
}

/// Writes an element's four-byte type signature followed by its four reserved zero bytes.
fn element_header(out: &mut Vec<u8>, type_sig: &[u8; 4]) {
    out.extend_from_slice(type_sig);
    out.extend_from_slice(&[0; 4]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::U8Fixed8;

    /// Builds an element: a four-byte type signature, four reserved bytes, then the payload.
    fn element(type_sig: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut e = Vec::with_capacity(8 + payload.len());
        e.extend_from_slice(type_sig);
        e.extend_from_slice(&[0; 4]);
        e.extend_from_slice(payload);
        e
    }

    #[test]
    fn decodes_xyz() {
        let mut payload = Vec::new();
        for raw in [0x0000_F6D6_i32, 0x0001_0000, 0x0000_D32D] {
            payload.extend_from_slice(&raw.to_be_bytes());
        }
        let TagData::Xyz(values) = decode_tag(&element(b"XYZ ", &payload)).unwrap() else {
            panic!("expected XYZ");
        };
        assert_eq!(
            values,
            vec![XyzNumber {
                x: S15Fixed16(0x0000_F6D6),
                y: S15Fixed16(0x0001_0000),
                z: S15Fixed16(0x0000_D32D),
            }]
        );
    }

    #[test]
    fn decodes_curve_variants() {
        let identity = element(b"curv", &0u32.to_be_bytes());
        assert_eq!(
            decode_tag(&identity).unwrap(),
            TagData::Curve(Curve::Identity)
        );

        let mut gamma = 1u32.to_be_bytes().to_vec();
        gamma.extend_from_slice(&0x0233u16.to_be_bytes()); // u8Fixed8 ≈ 2.2
        assert_eq!(
            decode_tag(&element(b"curv", &gamma)).unwrap(),
            TagData::Curve(Curve::Gamma(U8Fixed8(0x0233)))
        );

        let mut sampled = 3u32.to_be_bytes().to_vec();
        for v in [0u16, 32768, 65535] {
            sampled.extend_from_slice(&v.to_be_bytes());
        }
        assert_eq!(
            decode_tag(&element(b"curv", &sampled)).unwrap(),
            TagData::Curve(Curve::Sampled(vec![0, 32768, 65535]))
        );
    }

    #[test]
    fn rejects_curve_with_bogus_count() {
        let bogus = element(b"curv", &0xFFFF_FFFFu32.to_be_bytes());
        assert!(decode_tag(&bogus).is_err());
    }

    #[test]
    fn decodes_parametric_curve() {
        let mut payload = 1u16.to_be_bytes().to_vec(); // function type 1
        payload.extend_from_slice(&[0, 0]); // reserved
        for v in [1.0, 1.0, 0.0] {
            payload.extend_from_slice(&S15Fixed16::from_f64(v).0.to_be_bytes());
        }
        let TagData::ParametricCurve(curve) = decode_tag(&element(b"para", &payload)).unwrap()
        else {
            panic!("expected parametric curve");
        };
        assert_eq!(curve.function_type, 1);
        assert_eq!(curve.params.len(), 3);
    }

    #[test]
    fn decodes_text_and_strips_nul() {
        let data = decode_tag(&element(b"text", b"Copyright\0")).unwrap();
        assert_eq!(data, TagData::Text("Copyright".to_owned()));
    }

    #[test]
    fn rejects_non_ascii_text() {
        assert!(decode_tag(&element(b"text", &[0x80, 0x00])).is_err());
    }

    #[test]
    fn decodes_date_time() {
        let mut payload = Vec::new();
        for v in [2026u16, 6, 14, 12, 30, 45] {
            payload.extend_from_slice(&v.to_be_bytes());
        }
        assert_eq!(
            decode_tag(&element(b"dtim", &payload)).unwrap(),
            TagData::DateTime(DateTime {
                year: 2026,
                month: 6,
                day: 14,
                hours: 12,
                minutes: 30,
                seconds: 45,
            })
        );
    }

    #[test]
    fn decodes_signature() {
        assert_eq!(
            decode_tag(&element(b"sig ", b"prtr")).unwrap(),
            TagData::Signature(Signature(*b"prtr"))
        );
    }

    #[test]
    fn decodes_s15fixed16_array() {
        let mut payload = Vec::new();
        for v in [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0] {
            payload.extend_from_slice(&S15Fixed16::from_f64(v).0.to_be_bytes());
        }
        let TagData::S15Fixed16Array(values) = decode_tag(&element(b"sf32", &payload)).unwrap()
        else {
            panic!("expected sf32 array");
        };
        assert_eq!(values.len(), 9);
        assert_eq!(values[0], S15Fixed16(0x0001_0000));
    }

    #[test]
    fn unknown_element_is_preserved_as_raw() {
        let raw = element(b"zzzz", b"payload");
        let TagData::Raw { type_sig, bytes } = decode_tag(&raw).unwrap() else {
            panic!("expected Raw");
        };
        assert_eq!(type_sig, Signature(*b"zzzz"));
        assert_eq!(bytes, raw); // byte-for-byte verbatim
    }

    #[test]
    fn short_element_has_zero_type_signature() {
        let TagData::Raw { type_sig, bytes } = decode_tag(&[1, 2]).unwrap() else {
            panic!("expected Raw");
        };
        assert_eq!(type_sig, Signature::ZERO);
        assert_eq!(bytes, vec![1, 2]);
    }

    #[test]
    fn simple_elements_round_trip_through_encode() {
        let cases = [
            TagData::Xyz(vec![XyzNumber::from_f64([0.9642, 1.0, 0.8249])]),
            TagData::Curve(Curve::Gamma(U8Fixed8(0x0233))),
            TagData::ParametricCurve(ParametricCurve {
                function_type: 0,
                params: vec![S15Fixed16::from_f64(2.2)],
            }),
            TagData::Text("hello".to_owned()),
            TagData::DateTime(DateTime {
                year: 2026,
                month: 6,
                day: 14,
                hours: 1,
                minutes: 2,
                seconds: 3,
            }),
            TagData::Signature(Signature(*b"prtr")),
            TagData::S15Fixed16Array(vec![S15Fixed16::from_f64(1.0), S15Fixed16::from_f64(-0.5)]),
            TagData::NamedColor2(crate::named_color::NamedColor2 {
                vendor_flags: 0,
                prefix: String::new(),
                suffix: String::new(),
                colors: Vec::new(),
            }),
        ];
        for data in cases {
            let mut out = Vec::new();
            encode_tag(&data, &mut out);
            assert_eq!(decode_tag(&out).unwrap(), data);
        }
    }

    #[test]
    fn raw_round_trips_through_encode() {
        let data = TagData::Raw {
            type_sig: Signature(*b"zzzz"),
            bytes: b"zzzz\x00\x00\x00\x00payload".to_vec(),
        };
        let mut out = Vec::new();
        encode_tag(&data, &mut out);
        assert_eq!(decode_tag(&out).unwrap(), data);
    }
}
