//! The 128-byte ICC profile header (ICC.1:2022 §7.2).

use gamut_core::{Error, Result};

use crate::bytes::{ByteReader, push_date_time, push_xyz_number};
use crate::primitives::{DateTime, Signature, XyzNumber};

/// The fixed 128-byte header that opens every ICC profile (ICC.1:2022 §7.2).
///
/// Records the profile's size, the device/connection colour spaces it relates, the version,
/// the default rendering intent, the PCS illuminant, and an MD5 identifier. Every field is modelled
/// (open-registry signatures kept raw, closed registries as enums) so a parse → serialize round-trip
/// is lossless; the 4-byte `acsp` magic at offset 36 is validated on read and re-emitted on write,
/// so it is not a field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProfileHeader {
    /// Total profile size in bytes (offset 0). Recomputed by the writer.
    pub size: u32,
    /// Preferred CMM signature (offset 4, e.g. `appl`), or [`Signature::ZERO`].
    pub preferred_cmm: Signature,
    /// Profile format version (offset 8).
    pub version: ProfileVersion,
    /// Device/profile class (offset 12).
    pub device_class: DeviceClass,
    /// The data (device, "A"-side) colour space (offset 16).
    pub data_color_space: ColorSpace,
    /// The profile connection space (offset 20). Usually `XYZ`/`Lab`, but a device link records
    /// its output device space here.
    pub pcs: ColorSpace,
    /// Profile creation date-time in UTC (offset 24); may be [`DateTime::ZERO`].
    pub created: DateTime,
    /// Primary platform signature (offset 40, e.g. `APPL`/`MSFT`), or [`Signature::ZERO`].
    pub platform: Signature,
    /// Profile flags (offset 44): bit 0 = embedded, bit 1 = cannot be used independently; the
    /// upper half is CMM-private. Stored raw.
    pub flags: u32,
    /// Device manufacturer signature (offset 48), or [`Signature::ZERO`].
    pub manufacturer: Signature,
    /// Device model signature (offset 52), or [`Signature::ZERO`].
    pub model: Signature,
    /// Device attributes (offset 56): reflective/transparency, glossy/matte, polarity, colour/BW
    /// in the low bits; the upper half is vendor-specific. Stored raw.
    pub attributes: u64,
    /// Default rendering intent (offset 64).
    pub rendering_intent: RenderingIntent,
    /// PCS illuminant (offset 68); the spec mandates D50 ≈ (0.9642, 1.0, 0.8249).
    pub pcs_illuminant: XyzNumber,
    /// Profile creator signature (offset 80), or [`Signature::ZERO`].
    pub creator: Signature,
    /// Profile ID (offset 84): an MD5 of the profile with certain fields zeroed, or all-zero if
    /// unset (see [`ProfileId::is_zero`]).
    pub profile_id: ProfileId,
    /// The 28 reserved bytes (offset 100). The spec requires zero; preserved verbatim so a
    /// round-trip reproduces even non-conformant inputs exactly.
    pub reserved: [u8; 28],
}

impl ProfileHeader {
    /// Parses the 128-byte header from the start of an ICC profile.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if `bytes` is shorter than 128 bytes, the `acsp` signature
    /// is missing, or a closed-registry field (device class, colour space, rendering intent) holds
    /// an unrecognized value.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < 128 {
            return Err(Error::InvalidInput(
                "icc: profile shorter than 128-byte header",
            ));
        }
        let mut r = ByteReader::new(bytes);
        let size = r.u32()?;
        let preferred_cmm = r.signature()?;
        let major = r.u8()?;
        let minor_bugfix = r.u8()?;
        r.skip(2)?; // the two reserved version bytes
        let version = ProfileVersion::from_bytes(major, minor_bugfix);
        let device_class = DeviceClass::from_signature(r.signature()?)?;
        let data_color_space = ColorSpace::from_signature(r.signature()?)?;
        let pcs = ColorSpace::from_signature(r.signature()?)?;
        let created = r.date_time()?;
        if r.signature()? != Signature(*b"acsp") {
            return Err(Error::InvalidInput("icc: missing 'acsp' profile signature"));
        }
        let platform = r.signature()?;
        let flags = r.u32()?;
        let manufacturer = r.signature()?;
        let model = r.signature()?;
        let attributes = r.u64()?;
        let rendering_intent = RenderingIntent::from_u32(r.u32()?)?;
        let pcs_illuminant = r.xyz_number()?;
        let creator = r.signature()?;
        let mut profile_id = [0u8; 16];
        profile_id.copy_from_slice(r.bytes(16)?);
        let mut reserved = [0u8; 28];
        reserved.copy_from_slice(r.bytes(28)?);
        Ok(Self {
            size,
            preferred_cmm,
            version,
            device_class,
            data_color_space,
            pcs,
            created,
            platform,
            flags,
            manufacturer,
            model,
            attributes,
            rendering_intent,
            pcs_illuminant,
            creator,
            profile_id: ProfileId(profile_id),
            reserved,
        })
    }

    /// Serializes the 128-byte header. The `size` field is written as stored; the profile writer
    /// patches it to the final length and (optionally) stamps the recomputed profile ID.
    pub(crate) fn write(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.size.to_be_bytes());
        out.extend_from_slice(&self.preferred_cmm.0);
        out.push(self.version.major);
        out.push(self.version.minor_bugfix_byte());
        out.extend_from_slice(&[0, 0]); // reserved version bytes
        out.extend_from_slice(&self.device_class.to_signature().0);
        out.extend_from_slice(&self.data_color_space.to_signature().0);
        out.extend_from_slice(&self.pcs.to_signature().0);
        push_date_time(out, self.created);
        out.extend_from_slice(b"acsp");
        out.extend_from_slice(&self.platform.0);
        out.extend_from_slice(&self.flags.to_be_bytes());
        out.extend_from_slice(&self.manufacturer.0);
        out.extend_from_slice(&self.model.0);
        out.extend_from_slice(&self.attributes.to_be_bytes());
        out.extend_from_slice(&self.rendering_intent.to_u32().to_be_bytes());
        push_xyz_number(out, self.pcs_illuminant);
        out.extend_from_slice(&self.creator.0);
        out.extend_from_slice(&self.profile_id.0);
        out.extend_from_slice(&self.reserved);
    }
}

/// An ICC profile format version, e.g. 4.4.0 or 2.1.0 (ICC.1:2022 §7.2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProfileVersion {
    /// Major version (the first version byte; `2` or `4` in practice).
    pub major: u8,
    /// Minor version (the high nibble of the second version byte).
    pub minor: u8,
    /// Bug-fix version (the low nibble of the second version byte).
    pub bugfix: u8,
}

impl ProfileVersion {
    /// Decodes the version from its two significant bytes (the other two are reserved).
    fn from_bytes(major: u8, minor_bugfix: u8) -> Self {
        Self {
            major,
            minor: minor_bugfix >> 4,
            bugfix: minor_bugfix & 0x0F,
        }
    }

    /// Re-encodes the second version byte: minor in the high nibble, bug-fix in the low nibble.
    /// (`+` rather than `|` — the nibbles are disjoint so they are equal, but `+` keeps the
    /// arithmetic mutation-testable.)
    fn minor_bugfix_byte(self) -> u8 {
        ((self.minor & 0x0F) << 4) + (self.bugfix & 0x0F)
    }
}

/// The profile/device class (ICC.1:2022 §7.2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DeviceClass {
    /// `scnr` — input device (scanner, camera).
    Input,
    /// `mntr` — display device (monitor).
    Display,
    /// `prtr` — output device (printer).
    Output,
    /// `link` — a device link (a fused device-to-device transform).
    DeviceLink,
    /// `spac` — a colour-space conversion profile.
    ColorSpace,
    /// `abst` — an abstract profile.
    Abstract,
    /// `nmcl` — a named-colour profile.
    NamedColor,
}

impl DeviceClass {
    /// Maps a class signature to its variant.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if the signature is not one of the seven defined classes.
    pub fn from_signature(sig: Signature) -> Result<Self> {
        Ok(match &sig.0 {
            b"scnr" => Self::Input,
            b"mntr" => Self::Display,
            b"prtr" => Self::Output,
            b"link" => Self::DeviceLink,
            b"spac" => Self::ColorSpace,
            b"abst" => Self::Abstract,
            b"nmcl" => Self::NamedColor,
            _ => return Err(Error::InvalidInput("icc: unknown device class")),
        })
    }

    /// The four-byte signature for this class.
    #[must_use]
    pub fn to_signature(self) -> Signature {
        Signature(match self {
            Self::Input => *b"scnr",
            Self::Display => *b"mntr",
            Self::Output => *b"prtr",
            Self::DeviceLink => *b"link",
            Self::ColorSpace => *b"spac",
            Self::Abstract => *b"abst",
            Self::NamedColor => *b"nmcl",
        })
    }
}

/// A colour-space signature (ICC.1:2022 §7.2.6), used for both the data colour space and the PCS.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColorSpace {
    /// `XYZ ` — CIE XYZ (a valid PCS).
    Xyz,
    /// `Lab ` — CIE L\*a\*b\* (a valid PCS).
    Lab,
    /// `Luv ` — CIE L\*u\*v\*.
    Luv,
    /// `YCbr` — YCbCr.
    YCbCr,
    /// `Yxy ` — CIE Yxy.
    Yxy,
    /// `RGB ` — RGB.
    Rgb,
    /// `GRAY` — grayscale.
    Gray,
    /// `HSV ` — HSV.
    Hsv,
    /// `HLS ` — HLS.
    Hls,
    /// `CMYK` — CMYK.
    Cmyk,
    /// `CMY ` — CMY.
    Cmy,
    /// `nCLR` — an `n`-colorant space, `n` in 2–15 (e.g. `3CLR`, `ACLR`).
    NColor(u8),
}

impl ColorSpace {
    /// Maps a colour-space signature to its variant.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if the signature is neither a defined space nor a valid
    /// `nCLR` (2–15 colorants).
    pub fn from_signature(sig: Signature) -> Result<Self> {
        let s = &sig.0;
        Ok(match s {
            b"XYZ " => Self::Xyz,
            b"Lab " => Self::Lab,
            b"Luv " => Self::Luv,
            b"YCbr" => Self::YCbCr,
            b"Yxy " => Self::Yxy,
            b"RGB " => Self::Rgb,
            b"GRAY" => Self::Gray,
            b"HSV " => Self::Hsv,
            b"HLS " => Self::Hls,
            b"CMYK" => Self::Cmyk,
            b"CMY " => Self::Cmy,
            _ if s[1] == b'C' && s[2] == b'L' && s[3] == b'R' => {
                let n = match s[0] {
                    d @ b'2'..=b'9' => d - b'0',
                    d @ b'A'..=b'F' => d - b'A' + 10,
                    _ => return Err(Error::InvalidInput("icc: unknown colour space")),
                };
                Self::NColor(n)
            }
            _ => return Err(Error::InvalidInput("icc: unknown colour space")),
        })
    }

    /// The four-byte signature for this colour space.
    #[must_use]
    pub fn to_signature(self) -> Signature {
        match self {
            Self::Xyz => Signature(*b"XYZ "),
            Self::Lab => Signature(*b"Lab "),
            Self::Luv => Signature(*b"Luv "),
            Self::YCbCr => Signature(*b"YCbr"),
            Self::Yxy => Signature(*b"Yxy "),
            Self::Rgb => Signature(*b"RGB "),
            Self::Gray => Signature(*b"GRAY"),
            Self::Hsv => Signature(*b"HSV "),
            Self::Hls => Signature(*b"HLS "),
            Self::Cmyk => Signature(*b"CMYK"),
            Self::Cmy => Signature(*b"CMY "),
            Self::NColor(n) => {
                let d = if n < 10 {
                    b'0'.wrapping_add(n)
                } else {
                    b'A'.wrapping_add(n.wrapping_sub(10))
                };
                Signature([d, b'C', b'L', b'R'])
            }
        }
    }
}

/// The default rendering intent (ICC.1:2022 §7.2.15).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RenderingIntent {
    /// `0` — perceptual.
    Perceptual,
    /// `1` — media-relative colorimetric.
    MediaRelativeColorimetric,
    /// `2` — saturation.
    Saturation,
    /// `3` — ICC-absolute colorimetric.
    IccAbsoluteColorimetric,
}

impl RenderingIntent {
    /// Maps the header's rendering-intent word to its variant.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] for values outside 0–3 (the upper 16 bits must be zero).
    pub fn from_u32(value: u32) -> Result<Self> {
        Ok(match value {
            0 => Self::Perceptual,
            1 => Self::MediaRelativeColorimetric,
            2 => Self::Saturation,
            3 => Self::IccAbsoluteColorimetric,
            _ => return Err(Error::InvalidInput("icc: invalid rendering intent")),
        })
    }

    /// The rendering-intent word for this variant.
    #[must_use]
    pub fn to_u32(self) -> u32 {
        match self {
            Self::Perceptual => 0,
            Self::MediaRelativeColorimetric => 1,
            Self::Saturation => 2,
            Self::IccAbsoluteColorimetric => 3,
        }
    }
}

/// The 16-byte profile identifier (ICC.1:2022 §7.2.18): an MD5 of the profile with certain fields
/// zeroed, or all-zero when unset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProfileId(pub [u8; 16]);

impl ProfileId {
    /// The all-zero ID, meaning "unset".
    pub const ZERO: ProfileId = ProfileId([0; 16]);

    /// Whether the ID is unset (all zero).
    #[must_use]
    pub fn is_zero(self) -> bool {
        self.0 == [0; 16]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::primitives::S15Fixed16;

    /// A valid 128-byte header with a distinct value in every field, so a parse test pins each
    /// field's byte offset.
    fn sample_header() -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&0x0000_1234_u32.to_be_bytes()); // 0: size
        b.extend_from_slice(b"appl"); // 4: preferred CMM
        b.extend_from_slice(&[0x04, 0x30, 0x00, 0x00]); // 8: version 4.3.0
        b.extend_from_slice(b"mntr"); // 12: device class
        b.extend_from_slice(b"RGB "); // 16: data colour space
        b.extend_from_slice(b"XYZ "); // 20: PCS
        for v in [2026u16, 6, 14, 12, 30, 45] {
            b.extend_from_slice(&v.to_be_bytes()); // 24: created
        }
        b.extend_from_slice(b"acsp"); // 36: magic
        b.extend_from_slice(b"APPL"); // 40: platform
        b.extend_from_slice(&0x0000_0001_u32.to_be_bytes()); // 44: flags
        b.extend_from_slice(b"MFGR"); // 48: manufacturer
        b.extend_from_slice(b"MODL"); // 52: model
        b.extend_from_slice(&0x0000_0001_0000_0002_u64.to_be_bytes()); // 56: attributes
        b.extend_from_slice(&1_u32.to_be_bytes()); // 64: rendering intent
        for raw in [0x0000_F6D6_i32, 0x0001_0000, 0x0000_D32D] {
            b.extend_from_slice(&raw.to_be_bytes()); // 68: PCS illuminant (D50)
        }
        b.extend_from_slice(b"crtr"); // 80: creator
        b.extend_from_slice(&(1u8..=16).collect::<Vec<_>>()); // 84: profile ID
        b.extend_from_slice(&[0u8; 28]); // 100: reserved
        assert_eq!(b.len(), 128);
        b
    }

    #[test]
    fn parses_every_field_at_its_offset() {
        let h = ProfileHeader::parse(&sample_header()).unwrap();
        assert_eq!(h.size, 0x0000_1234);
        assert_eq!(h.preferred_cmm, Signature(*b"appl"));
        assert_eq!(
            h.version,
            ProfileVersion {
                major: 4,
                minor: 3,
                bugfix: 0
            }
        );
        assert_eq!(h.device_class, DeviceClass::Display);
        assert_eq!(h.data_color_space, ColorSpace::Rgb);
        assert_eq!(h.pcs, ColorSpace::Xyz);
        assert_eq!(
            h.created,
            DateTime {
                year: 2026,
                month: 6,
                day: 14,
                hours: 12,
                minutes: 30,
                seconds: 45
            }
        );
        assert_eq!(h.platform, Signature(*b"APPL"));
        assert_eq!(h.flags, 1);
        assert_eq!(h.manufacturer, Signature(*b"MFGR"));
        assert_eq!(h.model, Signature(*b"MODL"));
        assert_eq!(h.attributes, 0x0000_0001_0000_0002);
        assert_eq!(
            h.rendering_intent,
            RenderingIntent::MediaRelativeColorimetric
        );
        assert_eq!(
            h.pcs_illuminant,
            XyzNumber {
                x: S15Fixed16(0x0000_F6D6),
                y: S15Fixed16(0x0001_0000),
                z: S15Fixed16(0x0000_D32D),
            }
        );
        assert_eq!(h.creator, Signature(*b"crtr"));
        assert_eq!(
            h.profile_id,
            ProfileId([1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16])
        );
        assert_eq!(h.reserved, [0u8; 28]);
    }

    #[test]
    fn device_class_signature_round_trip() {
        for class in [
            DeviceClass::Input,
            DeviceClass::Display,
            DeviceClass::Output,
            DeviceClass::DeviceLink,
            DeviceClass::ColorSpace,
            DeviceClass::Abstract,
            DeviceClass::NamedColor,
        ] {
            assert_eq!(
                DeviceClass::from_signature(class.to_signature()).unwrap(),
                class
            );
        }
    }

    #[test]
    fn color_space_signature_round_trip() {
        let mut spaces = vec![
            ColorSpace::Xyz,
            ColorSpace::Lab,
            ColorSpace::Luv,
            ColorSpace::YCbCr,
            ColorSpace::Yxy,
            ColorSpace::Rgb,
            ColorSpace::Gray,
            ColorSpace::Hsv,
            ColorSpace::Hls,
            ColorSpace::Cmyk,
            ColorSpace::Cmy,
        ];
        spaces.extend((2..=15).map(ColorSpace::NColor));
        for space in spaces {
            assert_eq!(
                ColorSpace::from_signature(space.to_signature()).unwrap(),
                space
            );
        }
        // Spot-check the nCLR signature encoding (decimal digit and hex letter).
        assert_eq!(ColorSpace::NColor(3).to_signature(), Signature(*b"3CLR"));
        assert_eq!(ColorSpace::NColor(10).to_signature(), Signature(*b"ACLR"));

        // Signatures that resemble `nCLR` but break exactly one of the `_CLR` constraints must be
        // rejected (each pins one conjunct of the recognition guard).
        for bad in [b"3xLR", b"3CxR", b"3CLx", b"3zzz", b"1CLR"] {
            assert!(
                ColorSpace::from_signature(Signature(*bad)).is_err(),
                "{:?} should be rejected",
                core::str::from_utf8(bad).unwrap()
            );
        }
    }

    #[test]
    fn profile_id_zero_detection() {
        assert!(ProfileId::ZERO.is_zero());
        let mut id = ProfileId::ZERO;
        id.0[7] = 1;
        assert!(!id.is_zero());
    }

    #[test]
    fn version_with_bugfix_round_trips_through_write() {
        let mut header = ProfileHeader::parse(&sample_header()).unwrap();
        header.version = ProfileVersion {
            major: 4,
            minor: 3,
            bugfix: 2,
        };
        let mut out = Vec::new();
        header.write(&mut out);
        let parsed = ProfileHeader::parse(&out).unwrap();
        assert_eq!(
            parsed.version,
            ProfileVersion {
                major: 4,
                minor: 3,
                bugfix: 2,
            }
        );
    }

    #[test]
    fn rendering_intent_round_trip_and_range() {
        for value in 0..=3 {
            assert_eq!(RenderingIntent::from_u32(value).unwrap().to_u32(), value);
        }
        assert!(RenderingIntent::from_u32(4).is_err());
    }

    #[test]
    fn rejects_malformed_headers() {
        // Too short.
        assert!(ProfileHeader::parse(&[0u8; 64]).is_err());
        // Bad 'acsp' magic at offset 36.
        let mut bad = sample_header();
        bad[36] = b'X';
        assert!(ProfileHeader::parse(&bad).is_err());
        // Rendering intent out of range at offset 64.
        let mut bad = sample_header();
        bad[67] = 9;
        assert!(ProfileHeader::parse(&bad).is_err());
        // Unknown colour space at offset 16.
        let mut bad = sample_header();
        bad[16..20].copy_from_slice(b"zzzz");
        assert!(ProfileHeader::parse(&bad).is_err());
        // Unknown device class at offset 12.
        let mut bad = sample_header();
        bad[12..16].copy_from_slice(b"zzzz");
        assert!(ProfileHeader::parse(&bad).is_err());
    }
}
