//! `cicpType` (ICC.1:2022 §10.3): coding-independent code points for video signal type
//! identification, added in ICC.1:2022 to let HDR/wide-gamut signalling ride in a profile.
//!
//! The four fields are ITU-T H.273 (ISO/IEC 23091-2) code points. gamut-icc records the raw code
//! points only; mapping them to concrete primaries/transfer functions is the job of
//! [`gamut_color`](https://docs.rs/gamut-color), so this type deliberately adds no colour-science
//! dependency.

use gamut_core::Result;

use crate::bytes::ByteReader;

/// A `cicpType` element (§10.3): the four H.273 code points identifying a video signal type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cicp {
    /// `ColourPrimaries` (H.273 / CICP).
    pub colour_primaries: u8,
    /// `TransferCharacteristics` (H.273 / CICP).
    pub transfer_characteristics: u8,
    /// `MatrixCoefficients` (H.273 / CICP). Shall be `0` for an RGB or XYZ data colour space.
    pub matrix_coefficients: u8,
    /// `VideoFullRangeFlag` (H.273 / CICP): `0` narrow (studio) range, `1` full range.
    pub video_full_range_flag: u8,
}

/// Decodes a `cicpType` element.
pub(crate) fn decode_cicp(element: &[u8]) -> Result<Cicp> {
    let mut r = ByteReader::at(element, 8)?;
    Ok(Cicp {
        colour_primaries: r.u8()?,
        transfer_characteristics: r.u8()?,
        matrix_coefficients: r.u8()?,
        video_full_range_flag: r.u8()?,
    })
}

/// Serializes a `cicpType` element (the inverse of [`decode_cicp`]).
pub(crate) fn encode_cicp(cicp: &Cicp, out: &mut Vec<u8>) {
    out.extend_from_slice(b"cicp");
    out.extend_from_slice(&[0; 4]);
    out.push(cicp.colour_primaries);
    out.push(cicp.transfer_characteristics);
    out.push(cicp.matrix_coefficients);
    out.push(cicp.video_full_range_flag);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_srgb_code_points() {
        // 1-13-0-1 — the IEC 61966-2-1 sRGB full-range RGB encoding (§10.3 examples).
        let mut e = b"cicp\x00\x00\x00\x00".to_vec();
        e.extend_from_slice(&[1, 13, 0, 1]);
        let cicp = decode_cicp(&e).unwrap();
        assert_eq!(
            cicp,
            Cicp {
                colour_primaries: 1,
                transfer_characteristics: 13,
                matrix_coefficients: 0,
                video_full_range_flag: 1,
            }
        );
    }

    #[test]
    fn round_trips_through_encode() {
        // 9-16-0-1 — PQ BT.2100 full-range R'G'B'.
        let cicp = Cicp {
            colour_primaries: 9,
            transfer_characteristics: 16,
            matrix_coefficients: 0,
            video_full_range_flag: 1,
        };
        let mut out = Vec::new();
        encode_cicp(&cicp, &mut out);
        assert_eq!(decode_cicp(&out).unwrap(), cicp);
    }

    #[test]
    fn rejects_truncated_element() {
        assert!(decode_cicp(b"cicp\x00\x00\x00\x00\x01\x0d").is_err());
    }
}
