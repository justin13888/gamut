//! The AVIF still-image encoder: RGB → identity planes → AV1 temporal unit → ISOBMFF container.

use gamut_av1::{Av1StillConfig, EncodedStill, encode_still_intra, encode_still_lossless_identity};
use gamut_color::Planar8;
use gamut_core::{Dimensions, EncodeImage, ImageRef, Result, Rgb8};
use gamut_isobmff::{
    ColourInformation, IsoBmffImage, Item, NclxColr, Property, PropertyKind, write,
};

/// The encoder's display-orientation transforms, applied by a reader at display time (the stored
/// pixels are unchanged). Maps to the `irot`/`imir` item properties.
#[derive(Debug, Clone, Copy, Default)]
struct ImageTransform {
    /// `irot` rotation in 90° steps (`0..=3`), anti-clockwise. `0` writes no `irot`.
    rotation_ccw: u8,
    /// `imir` mirror axis: `Some(0)` vertical (left↔right), `Some(1)` horizontal (top↔bottom).
    mirror_axis: Option<u8>,
}

/// Encodes images to AVIF still images.
///
/// 8-bit RGB in, mapped to AV1 identity-matrix 4:4:4. By default the encode is **lossless**;
/// [`AvifEncoder::with_qindex`] selects a lossy quantizer (`base_q_idx`, `1..=255`). Use
/// Encode via the [`EncodeImage<Rgb8>`](gamut_core::EncodeImage) trait, taking a typed
/// [`ImageRef`]. [`AvifEncoder::with_rotation_ccw`] /
/// [`AvifEncoder::with_mirror`] add `irot`/`imir` display-orientation transforms.
#[derive(Debug, Clone, Copy, Default)]
pub struct AvifEncoder {
    /// AV1 `base_q_idx`: `0` is lossless, `1..=255` is lossy intra (higher = more quantization).
    qindex: u8,
    /// Optional `irot`/`imir` display-orientation transforms.
    transform: ImageTransform,
}

impl AvifEncoder {
    /// Creates an encoder (lossless by default).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the AV1 quantizer (`base_q_idx`): `0` is lossless, `1..=255` is lossy intra (higher
    /// quantizes more aggressively for smaller files). Returns the updated encoder for chaining.
    #[must_use]
    pub fn with_qindex(mut self, qindex: u8) -> Self {
        self.qindex = qindex;
        self
    }

    /// Adds an `irot` display rotation of `quarter_turns × 90°` applied anti-clockwise (the value is
    /// taken modulo 4, so `0` clears it). The stored pixels are unchanged — a reader rotates at
    /// display time — so this records e.g. a camera's EXIF orientation without re-encoding. Returns
    /// the updated encoder for chaining.
    #[must_use]
    pub fn with_rotation_ccw(mut self, quarter_turns: u8) -> Self {
        self.transform.rotation_ccw = quarter_turns % 4;
        self
    }

    /// Adds an `imir` display mirror: `axis = 0` mirrors about a vertical axis (left↔right), `1`
    /// about a horizontal axis (top↔bottom). The stored pixels are unchanged. Returns the updated
    /// encoder for chaining.
    #[must_use]
    pub fn with_mirror(mut self, axis: u8) -> Self {
        self.transform.mirror_axis = Some(axis & 1);
        self
    }
}

/// The 4-byte `AV1CodecConfigurationRecord` body (empty `configOBUs`) stamped into the `av1C`
/// property (AV1-ISOBMFF v1.3.0 §2.3.3/§2.3.4). Every field mirrors the AV1 sequence header.
fn av1c_record(c: &Av1StillConfig) -> [u8; 4] {
    [
        0x81, // marker = 1, version = 1
        (c.seq_profile << 5) + (c.seq_level_idx_0 & 0x1f),
        (c.seq_tier_0 << 7)
            + (u8::from(c.high_bitdepth) << 6)
            + (u8::from(c.twelve_bit) << 5)
            + (u8::from(c.monochrome) << 4)
            + (c.chroma_subsampling_x << 3)
            + (c.chroma_subsampling_y << 2)
            + (c.chroma_sample_position & 0x3),
        0x00, // reserved(3)=0, initial_presentation_delay_present(1)=0, reserved(4)=0
    ]
}

/// Wraps the encoded AV1 temporal unit in the AVIF container, stamping `av1C`/`colr`/`ispe`/`pixi`
/// from the AV1 configuration so the cross-box consistency requirements hold by construction
/// (AVIF v1.2.0 §2.2, AV1-ISOBMFF v1.3.0 §2.3.4).
fn build_avif(still: &EncodedStill, dims: Dimensions, transform: ImageTransform) -> Vec<u8> {
    let c = &still.config;
    // av1C is essential; ispe/pixi/colr are descriptive. Order fixes the ipco/ipma indices.
    let mut properties = vec![
        Property {
            essential: true,
            kind: PropertyKind::CodecConfiguration {
                kind: *b"av1C",
                data: av1c_record(c).to_vec(),
            },
        },
        Property {
            essential: false,
            kind: PropertyKind::ImageSpatialExtents {
                width: dims.width,
                height: dims.height,
            },
        },
        Property {
            essential: false,
            kind: PropertyKind::PixelInformation {
                bits_per_channel: vec![8, 8, 8],
            },
        },
        Property {
            essential: false,
            kind: PropertyKind::Colour(ColourInformation::Nclx(NclxColr {
                colour_primaries: c.color_primaries,
                transfer_characteristics: c.transfer_characteristics,
                matrix_coefficients: c.matrix_coefficients,
                full_range: c.full_range,
            })),
        },
    ];
    // Transformative properties are essential (MIAF §7.3.6.7); applied irot-then-imir.
    if transform.rotation_ccw != 0 {
        properties.push(Property {
            essential: true,
            kind: PropertyKind::Rotation(transform.rotation_ccw),
        });
    }
    if let Some(axis) = transform.mirror_axis {
        properties.push(Property {
            essential: true,
            kind: PropertyKind::Mirror(axis),
        });
    }
    let image = IsoBmffImage {
        major_brand: *b"avif",
        minor_version: 0,
        compatible_brands: vec![*b"avif", *b"mif1", *b"miaf", *b"MA1A"],
        primary_item_id: 1,
        items: vec![Item {
            id: 1,
            item_type: *b"av01",
            name: String::new(),
            properties,
            payload: still.obus.clone(),
        }],
    };
    write(&image)
}

impl EncodeImage<Rgb8> for AvifEncoder {
    /// Maps the RGB image to AV1 identity 4:4:4 planes and wraps the temporal unit in an AVIF file.
    fn encode_image(&self, image: ImageRef<'_, Rgb8>, out: &mut Vec<u8>) -> Result<usize> {
        let dims = image.dimensions();
        let planes = Planar8::from_rgb8_identity_view(image);
        let still = if self.qindex == 0 {
            encode_still_lossless_identity(&planes)?
        } else {
            encode_still_intra(&planes, self.qindex)?.0
        };
        let file = build_avif(&still, dims, self.transform);
        out.extend_from_slice(&file);
        Ok(file.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn av1c_record_encodes_every_field() {
        // Distinct, non-zero values in every field so each shift, mask, and `+` is observable (a
        // zero term would hide its operator: `0 + x == 0 - x`, `0 << n == 0 >> n`).
        let c = Av1StillConfig {
            seq_profile: 5,        // 0b101
            seq_level_idx_0: 0x15, // 0b10101
            seq_tier_0: 1,
            high_bitdepth: true,
            twelve_bit: true,
            monochrome: true,
            chroma_subsampling_x: 1,
            chroma_subsampling_y: 1,
            chroma_sample_position: 2, // 0b10
            // colr fields are irrelevant to av1C but needed to build the config.
            color_primaries: 2,
            transfer_characteristics: 3,
            matrix_coefficients: 5,
            full_range: true,
        };
        // marker/version 0x81; (seq_profile<<5)+(level&0x1f) = 0xA0+0x15 = 0xB5; the flags byte sets
        // tier/high_bitdepth/twelve_bit/monochrome/subsampling_x/_y plus chroma position 2:
        // 0x80+0x40+0x20+0x10+0x08+0x04+0x02 = 0xFE; trailing reserved 0x00.
        assert_eq!(av1c_record(&c), [0x81, 0xB5, 0xFE, 0x00]);
    }

    fn encode(w: u32, h: u32) -> Vec<u8> {
        let mut rgb = vec![0u8; (w * h * 3) as usize];
        for (i, b) in rgb.iter_mut().enumerate() {
            *b = (i * 37) as u8;
        }
        let mut out = Vec::new();
        let dims = Dimensions {
            width: w,
            height: h,
        };
        AvifEncoder::new()
            .encode_image(ImageRef::<Rgb8>::new(&rgb, dims).unwrap(), &mut out)
            .unwrap();
        out
    }

    #[test]
    fn produces_valid_avif_container() {
        let f = encode(40, 24);
        assert_eq!(&f[4..8], b"ftyp");
        for fourcc in [
            b"meta", b"av1C", b"ispe", b"pixi", b"colr", b"mdat", b"av01",
        ] {
            assert!(f.windows(4).any(|w| w == fourcc), "missing box {fourcc:?}");
        }
    }

    #[test]
    fn lossy_produces_valid_avif_container() {
        // A lossy quantizer still produces the same well-formed container; only the mdat payload
        // (the AV1 OBUs) differs. Exercises the with_qindex path across quantizer contexts.
        let mut rgb = vec![0u8; 48 * 32 * 3];
        for (i, b) in rgb.iter_mut().enumerate() {
            *b = (i * 29) as u8;
        }
        for q in [4u8, 40, 200] {
            let mut out = Vec::new();
            let n = AvifEncoder::new()
                .with_qindex(q)
                .encode_image(
                    ImageRef::<Rgb8>::new(
                        &rgb,
                        Dimensions {
                            width: 48,
                            height: 32,
                        },
                    )
                    .unwrap(),
                    &mut out,
                )
                .unwrap();
            assert_eq!(n, out.len());
            assert_eq!(&out[4..8], b"ftyp");
            for fourcc in [b"meta", b"av1C", b"ispe", b"mdat", b"av01"] {
                assert!(
                    out.windows(4).any(|w| w == fourcc),
                    "missing box {fourcc:?}"
                );
            }
        }
    }

    #[test]
    fn ispe_matches_dimensions() {
        let (w, h) = (37u32, 19u32);
        let f = encode(w, h);
        let pos = f.windows(4).position(|x| x == b"ispe").unwrap();
        let body = pos + 4 + 4; // skip 'ispe' fourcc + FullBox version/flags
        let rw = u32::from_be_bytes([f[body], f[body + 1], f[body + 2], f[body + 3]]);
        let rh = u32::from_be_bytes([f[body + 4], f[body + 5], f[body + 6], f[body + 7]]);
        assert_eq!((rw, rh), (w, h));
    }

    #[test]
    fn rejects_wrong_length() {
        // The wrong-length buffer can't even be wrapped in an ImageRef for the encoder.
        let r = ImageRef::<Rgb8>::new(
            &[0; 10],
            Dimensions {
                width: 4,
                height: 4,
            },
        );
        assert!(r.is_err());
    }

    #[test]
    fn appends_without_clobbering() {
        let mut out = vec![0xAA, 0xBB];
        let rgb = vec![128u8; 4 * 4 * 3];
        let n = AvifEncoder::new()
            .encode_image(
                ImageRef::<Rgb8>::new(
                    &rgb,
                    Dimensions {
                        width: 4,
                        height: 4,
                    },
                )
                .unwrap(),
                &mut out,
            )
            .unwrap();
        assert_eq!(out.len(), 2 + n);
        assert_eq!(&out[0..2], &[0xAA, 0xBB]);
    }

    fn encode_with(enc: AvifEncoder, w: u32, h: u32) -> Vec<u8> {
        let mut rgb = vec![0u8; (w * h * 3) as usize];
        for (i, b) in rgb.iter_mut().enumerate() {
            *b = (i * 37) as u8;
        }
        let mut out = Vec::new();
        let dims = Dimensions {
            width: w,
            height: h,
        };
        enc.encode_image(ImageRef::<Rgb8>::new(&rgb, dims).unwrap(), &mut out)
            .unwrap();
        out
    }

    #[test]
    fn with_rotation_ccw_emits_irot_and_normalizes_mod_four() {
        // A non-zero rotation emits an `irot` whose body byte is the angle. `irot` lives in `meta`,
        // which precedes `mdat`, so the first occurrence is the property box (not stray OBU bytes).
        let f = encode_with(AvifEncoder::new().with_rotation_ccw(1), 4, 4);
        let p = f
            .windows(4)
            .position(|w| w == b"irot")
            .expect("irot present");
        assert_eq!(f[p + 4] & 0x03, 1, "irot angle = 1");
        // 4 ≡ 0 (mod 4) clears the rotation, so no `irot` is written.
        let f0 = encode_with(AvifEncoder::new().with_rotation_ccw(4), 4, 4);
        assert!(
            !f0.windows(4).any(|w| w == b"irot"),
            "rotation 4 ≡ 0 ⇒ no irot"
        );
    }

    #[test]
    fn with_mirror_emits_imir_axis() {
        for axis in [0u8, 1] {
            let f = encode_with(AvifEncoder::new().with_mirror(axis), 4, 4);
            let p = f
                .windows(4)
                .position(|w| w == b"imir")
                .expect("imir present");
            assert_eq!(f[p + 4] & 0x01, axis, "imir axis = {axis}");
            assert!(!f.windows(4).any(|w| w == b"irot"), "mirror only ⇒ no irot");
        }
    }
}
