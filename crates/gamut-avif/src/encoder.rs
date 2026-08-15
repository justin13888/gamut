//! The AVIF still-image encoder: RGB → identity planes → AV1 temporal unit → ISOBMFF container.

use std::sync::{Arc, Mutex};

use gamut_av1::{Av1Colour, Av1StillConfig, EncodedStill, encode_still_intra_with};
use gamut_color::{ColorRange, MatrixCoefficients, Planar8, YcbcrMatrix};
use gamut_core::{Dimensions, EncodeImage, ImageRef, Result, Rgb8};
use gamut_isobmff::{
    ColourInformation, IsoBmffImage, Item, NclxColr, Property, PropertyKind, write,
};

use crate::backend::{Av1EncodeRequest, Av1StillEncoder, BackendSlot};
use crate::config::{AvifConfig, AvifMode};
use crate::transform::{Mirror, Rotation};

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
/// 8-bit RGB in, mapped to AV1 identity-matrix 4:4:4 planes. Construct with [`AvifEncoder::new`]
/// (lossless), [`AvifEncoder::lossless`], or [`AvifEncoder::lossy`], then encode via the
/// [`EncodeImage<Rgb8>`](gamut_core::EncodeImage) trait, taking a typed [`ImageRef`].
/// [`AvifEncoder::with_rotation`] / [`AvifEncoder::with_mirror`] add `irot`/`imir`
/// display-orientation transforms.
///
/// The AV1 codestream comes from `gamut-av1` by default; [`AvifEncoder::push_backend`] registers
/// alternate [`Av1StillEncoder`] backends ahead of it (see [`crate::backend`] for the fallback
/// contract).
#[derive(Clone)]
pub struct AvifEncoder {
    /// Lossless/lossy mode and the lossy quality factor.
    config: AvifConfig,
    /// Optional `irot`/`imir` display-orientation transforms.
    transform: ImageTransform,
    /// Pluggable AV1 still-encode backends, tried in push order before the `gamut-av1` tail.
    /// Shared (not copied) by [`Clone`] — see [`AvifEncoder::push_backend`].
    backends: Vec<BackendSlot>,
}

/// Hand-written because a backend registry is not [`Debug`]: the backends are opaque
/// caller-supplied objects, so they are summarized by count. Every other field is printed as the
/// derive would.
impl std::fmt::Debug for AvifEncoder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AvifEncoder")
            .field("config", &self.config)
            .field("transform", &self.transform)
            .field("backends", &self.backends.len())
            .finish()
    }
}

impl Default for AvifEncoder {
    /// The default encoder is **lossless** — defined as [`AvifEncoder::lossless`].
    fn default() -> Self {
        Self::lossless()
    }
}

impl AvifEncoder {
    /// Creates an encoder with the default configuration; equivalent to [`AvifEncoder::lossless`].
    #[must_use]
    pub fn new() -> Self {
        Self::lossless()
    }

    /// Creates an encoder that produces a **lossless** still image — the decoded output is bit-exact
    /// to the input. This is the default mode, so [`AvifEncoder::new`] and [`AvifEncoder::default`]
    /// return the same encoder; it exists to pair with [`AvifEncoder::lossy`] and make intent
    /// explicit at the call site.
    #[must_use]
    pub fn lossless() -> Self {
        Self {
            config: AvifConfig {
                mode: AvifMode::Lossless,
                // Quality is ignored in lossless mode; carry the config's default for `config()`.
                ..AvifConfig::default()
            },
            transform: ImageTransform::default(),
            backends: Vec::new(),
        }
    }

    /// Creates an encoder that produces a **lossy** still image at the given `quality` (`0..=100`,
    /// higher = larger output, closer to the source; values above `100` are clamped).
    /// Lossy stills are coded in **BT.709 YCbCr** by default (full range, 4:4:4): the luma–chroma
    /// decorrelation is worth a large fraction of the bitrate and costs nothing in coding tools.
    /// Override it with [`AvifEncoder::with_matrix`].
    #[must_use]
    pub fn lossy(quality: u8) -> Self {
        Self {
            config: AvifConfig {
                mode: AvifMode::Lossy,
                quality,
                matrix: MatrixCoefficients::Bt709,
                ..AvifConfig::default()
            },
            transform: ImageTransform::default(),
            backends: Vec::new(),
        }
    }

    /// Returns a snapshot of the encoder's configuration.
    #[must_use]
    pub fn config(&self) -> AvifConfig {
        self.config
    }

    /// Selects the CICP matrix the samples are coded through — the colour half of the
    /// space/quality tradeoff.
    ///
    /// Supported: [`MatrixCoefficients::Identity`] (R'G'B' as GBR planes, no decorrelation),
    /// [`MatrixCoefficients::Bt601`], [`MatrixCoefficients::Bt709`] (the lossy default) and
    /// [`MatrixCoefficients::Bt2020Ncl`]. Any other code point is rejected at encode time.
    ///
    /// **Ignored by [`AvifMode::Lossless`]**, which always uses the identity matrix — an 8-bit
    /// YCbCr round trip is not bit-exact, so a lossless encode through a luma–chroma matrix would
    /// not be lossless. This mirrors how lossless already ignores `quality`.
    ///
    /// Colour **primaries** and the transfer function stay BT.709 / sRGB; selecting those is part
    /// of the deferred colour-metadata surface (`STATUS.md`). A BT.2020 *matrix* with BT.709
    /// primaries is legal CICP — the matrix is a coding choice, independent of the gamut — but a
    /// caller wanting a true BT.2020 image needs that surface, not this knob.
    #[must_use]
    pub fn with_matrix(mut self, matrix: MatrixCoefficients) -> Self {
        self.config.matrix = matrix;
        self
    }

    /// Selects the signal range the coded samples occupy, signalled in `colr` and the AV1 sequence
    /// header. Defaults to [`ColorRange::Full`], the AVIF ecosystem's default.
    ///
    /// Ignored by [`AvifMode::Lossless`] (and by an identity-matrix encode generally): AV1's
    /// §5.5.2 sRGB shortcut infers full range for BT.709/sRGB/identity and codes no bit for it.
    #[must_use]
    pub fn with_color_range(mut self, range: ColorRange) -> Self {
        self.config.range = range;
        self
    }

    /// The colour signalling and plane layout this encoder's configuration selects.
    ///
    /// Lossless pins identity/full regardless of the configured matrix and range — see
    /// [`AvifEncoder::with_matrix`].
    fn colour(&self) -> Av1Colour {
        match self.config.mode {
            AvifMode::Lossless => Av1Colour::default(),
            _ => Av1Colour {
                matrix: self.config.matrix,
                range: self.config.range,
                ..Av1Colour::default()
            },
        }
    }

    /// Records an `irot` display [`Rotation`] applied by a reader (the stored pixels are unchanged,
    /// so this captures e.g. a camera's EXIF orientation without re-encoding rotated samples).
    /// [`Rotation::None`] writes no `irot`. Returns the updated encoder for chaining.
    #[must_use]
    pub fn with_rotation(mut self, rotation: Rotation) -> Self {
        self.transform.rotation_ccw = rotation.quarter_turns();
        self
    }

    /// Records an `imir` display [`Mirror`] applied by a reader (the stored pixels are unchanged).
    /// Returns the updated encoder for chaining.
    #[must_use]
    pub fn with_mirror(mut self, mirror: Mirror) -> Self {
        self.transform.mirror_axis = Some(mirror.axis());
        self
    }

    /// Registers an [`Av1StillEncoder`] backend for the AV1 codestream, returning `&mut self` for
    /// chaining.
    ///
    /// Backends are tried in **push order**, ahead of the built-in `gamut-av1` encoder, which is
    /// the implicit tail: for each encode the first backend whose
    /// [`supports`](Av1StillEncoder::supports) returns `true` produces the codestream, a backend
    /// that returns `false` is skipped, and if every backend declines the built-in encoder runs. A
    /// backend that accepts a job and then fails propagates its error — the built-in encoder is
    /// **not** used as a silent fallback, because substituting a different encoder would change
    /// the output bytes unpredictably. An encoder with no backends is byte-for-byte the encoder
    /// this crate shipped before backends existed.
    ///
    /// # Cloning shares backends
    ///
    /// Backends are held behind [`Arc`]`<`[`Mutex`]`<…>>`, so [`clone`](Clone::clone)ing an
    /// `AvifEncoder` yields an encoder that drives **the same** backend objects, not copies. A
    /// stateful backend therefore observes the encodes of every clone, and clones made *before* a
    /// `push_backend` call do not gain the new backend (the registry vector itself is per-encoder).
    ///
    /// ```
    /// use gamut_avif::{Av1EncodeRequest, Av1StillEncoder, AvifEncoder};
    /// use gamut_color::Planar8;
    ///
    /// // A backend that declines everything, so encoding still uses the built-in encoder.
    /// struct Decline;
    /// impl Av1StillEncoder for Decline {
    ///     fn supports(&mut self, _req: &Av1EncodeRequest) -> bool { false }
    ///     fn encode_still(
    ///         &mut self,
    ///         _req: &Av1EncodeRequest,
    ///         _planes: &Planar8,
    ///     ) -> gamut_core::Result<Vec<u8>> {
    ///         unreachable!("declined")
    ///     }
    /// }
    ///
    /// let mut encoder = AvifEncoder::new();
    /// encoder.push_backend(Decline);
    /// ```
    pub fn push_backend(&mut self, backend: impl Av1StillEncoder + 'static) -> &mut Self {
        self.backends.push(Arc::new(Mutex::new(backend)));
        self
    }
}

/// The 4-byte `AV1CodecConfigurationRecord` body (empty `configOBUs`) stamped into the `av1C`
/// property (AV1-ISOBMFF v1.3.0 §2.3.3/§2.3.4). Every field mirrors the AV1 sequence header.
/// Crate-visible so the `av1c` module's tests can pin writer/reader coherence.
pub(crate) fn av1c_record(c: &Av1StillConfig) -> [u8; 4] {
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
fn build_avif(
    still: &EncodedStill,
    dims: Dimensions,
    transform: ImageTransform,
) -> Result<Vec<u8>> {
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
            content_type: None,
            content_encoding: None,
            hidden: false,
            references: vec![],
            properties,
            payload: still.obus.clone(),
        }],
        groups: vec![],
    };
    write(&image)
}

/// Maps a `0..=100` quality to an AV1 `base_q_idx` (`1..=255`); higher quality → lower index (less
/// quantization). `base_q_idx 0` (the lossless WHT path) is reserved for [`AvifEncoder::lossless`],
/// so the lossy path stays on the DCT pipeline — `lossy(100)` is the finest lossy quantizer, not
/// lossless. Finer rate control (target size/metric) is future work (see `STATUS.md`).
fn quality_to_quant(quality: u8) -> u8 {
    let q = u32::from(quality.min(100));
    (((100 - q) * 255 / 100) as u8).max(1)
}

impl EncodeImage<Rgb8> for AvifEncoder {
    /// Maps the RGB image to AV1 4:4:4 planes — identity GBR, or YCbCr through the configured
    /// matrix — and wraps the temporal unit in an AVIF file.
    fn encode_image(&self, image: ImageRef<'_, Rgb8>, out: &mut Vec<u8>) -> Result<usize> {
        let dims = image.dimensions();
        let colour = self.colour();
        let planes = match colour.matrix {
            MatrixCoefficients::Identity => Planar8::from_rgb8_identity_view(image),
            matrix => {
                // Rejects a matrix with no luma–chroma transform (Unspecified, YCgCo) before any
                // bytes are written.
                let m = YcbcrMatrix::new(matrix, colour.range)?;
                Planar8::from_rgb8_matrix_view(image, m)
            }
        };
        // base_q_idx 0 is the lossless path; encode_still_intra(_, 0) is exactly what
        // encode_still_lossless_identity does, so a single call covers both modes.
        let base_q_idx = match self.config.mode {
            AvifMode::Lossless => 0,
            AvifMode::Lossy => quality_to_quant(self.config.quality),
        };
        // Pluggable backends first, in push order; `gamut-av1` is the implicit tail when every
        // backend declines (and the only path taken by an encoder with no backends, which is why
        // the default output is byte-identical to the pre-backend encoder).
        let request = Av1EncodeRequest::new(dims, base_q_idx, colour);
        let still = match crate::backend::run_backends(&self.backends, &request, &planes)? {
            Some(obus) => crate::backend::still_from_backend_obus(obus, dims, colour)?,
            None => encode_still_intra_with(&planes, base_q_idx, colour)?.0,
        };
        let file = build_avif(&still, dims, self.transform)?;
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

    #[test]
    fn quality_maps_to_quant() {
        // 0..=100, higher quality = lower base_q_idx (less quantization). base_q_idx 0 is reserved
        // for the lossless path, so the lossy mapping floors at 1 and never returns 0.
        assert_eq!(
            quality_to_quant(100),
            1,
            "best quality = finest lossy quantizer"
        );
        assert_eq!(
            quality_to_quant(0),
            255,
            "worst quality = coarsest quantizer"
        );
        assert_eq!(quality_to_quant(50), 127);
        assert_eq!(
            quality_to_quant(200),
            1,
            "out-of-range quality is clamped to 100"
        );
        // The constructors set the mode; lossless never consults the quality field.
        assert_eq!(AvifEncoder::lossless().config().mode, AvifMode::Lossless);
        let lossy = AvifEncoder::lossy(80).config();
        assert_eq!(lossy.mode, AvifMode::Lossy);
        assert_eq!(lossy.quality, 80);
    }

    #[test]
    fn container_carries_av1_config_and_layout() {
        use gamut_isobmff::{ColourInformation, PropertyKind, read};
        // Both modes wrap the AV1 unit in the same well-formed container; only the mdat payload
        // differs. Parsing it back (gamut-isobmff round-trips its own output) pins the brands, the
        // primary `av01` item, and the av1C-derived `ispe`/`pixi`/`colr` the encoder stamps — none
        // of which a box-presence check would catch if a field were wrong.
        // `(encoder, expected matrix_coefficients, expected full_range)`: lossless is pinned to
        // identity/full, lossy defaults to BT.709/full, and both knobs override the lossy defaults.
        let cases = [
            (AvifEncoder::lossless(), 0u16, true),
            (AvifEncoder::lossy(50), 1, true),
            (
                AvifEncoder::lossy(50).with_matrix(MatrixCoefficients::Bt601),
                6,
                true,
            ),
            // Studio range reaches `colr` — and can only be signalled outside the §5.5.2 sRGB
            // shortcut, which is why it pairs with a real matrix.
            (
                AvifEncoder::lossy(50).with_color_range(ColorRange::Limited),
                1,
                false,
            ),
            // …but neither knob applies on the lossless path, which ignores them as it ignores
            // quality: an 8-bit YCbCr round trip is not bit-exact, and studio range discards codes.
            (
                AvifEncoder::lossless()
                    .with_matrix(MatrixCoefficients::Bt709)
                    .with_color_range(ColorRange::Limited),
                0,
                true,
            ),
        ];
        for (enc, want_matrix, want_full_range) in cases {
            let img = read(&encode_with(enc, 34, 18)).expect("emitted AVIF parses");
            assert_eq!(img.major_brand, *b"avif");
            for brand in [*b"avif", *b"mif1", *b"miaf", *b"MA1A"] {
                assert!(
                    img.compatible_brands.contains(&brand),
                    "missing brand {brand:?}"
                );
            }
            assert_eq!(img.primary_item_id, 1);
            let item = &img.items[0];
            assert_eq!(item.item_type, *b"av01");
            let props = &item.properties;
            let ispe = props.iter().find_map(|p| match p.kind {
                PropertyKind::ImageSpatialExtents { width, height } => Some((width, height)),
                _ => None,
            });
            assert_eq!(ispe, Some((34, 18)), "ispe = display dimensions");
            let pixi = props.iter().find_map(|p| match &p.kind {
                PropertyKind::PixelInformation { bits_per_channel } => {
                    Some(bits_per_channel.clone())
                }
                _ => None,
            });
            assert_eq!(pixi, Some(vec![8u8, 8, 8]), "three 8-bit channels (4:4:4)");
            let nclx = props
                .iter()
                .find_map(|p| match &p.kind {
                    PropertyKind::Colour(ColourInformation::Nclx(n)) => Some(n),
                    _ => None,
                })
                .expect("colr nclx present");
            // Primaries and transfer stay BT.709 / sRGB (the colour-metadata surface is deferred);
            // the matrix and range are whatever the configuration selected (AVIF v1.2.0 §2.2;
            // mc = 0 additionally requires 4:4:4 full range).
            assert_eq!(nclx.colour_primaries, 1);
            assert_eq!(nclx.transfer_characteristics, 13);
            assert_eq!(nclx.matrix_coefficients, want_matrix);
            assert_eq!(nclx.full_range, want_full_range);
        }
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
    fn with_rotation_emits_irot_and_none_is_omitted() {
        // A rotation emits an `irot` whose body byte is the angle. `irot` lives in `meta`, which
        // precedes `mdat`, so the first occurrence is the property box (not stray OBU bytes).
        let f = encode_with(AvifEncoder::new().with_rotation(Rotation::Ccw90), 4, 4);
        let p = f
            .windows(4)
            .position(|w| w == b"irot")
            .expect("irot present");
        assert_eq!(f[p + 4] & 0x03, 1, "Ccw90 ⇒ irot angle = 1");
        // Rotation::None writes no `irot`.
        let f0 = encode_with(AvifEncoder::new().with_rotation(Rotation::None), 4, 4);
        assert!(
            !f0.windows(4).any(|w| w == b"irot"),
            "Rotation::None ⇒ no irot"
        );
    }

    #[test]
    fn with_mirror_emits_imir_axis() {
        // ISO/IEC 23008-12:2022 §6.5.12: axis 1 exchanges left/right, axis 0 top/bottom.
        for (mirror, axis) in [(Mirror::LeftRight, 1u8), (Mirror::TopBottom, 0)] {
            let f = encode_with(AvifEncoder::new().with_mirror(mirror), 4, 4);
            let p = f
                .windows(4)
                .position(|w| w == b"imir")
                .expect("imir present");
            assert_eq!(f[p + 4] & 0x01, axis, "{mirror:?} ⇒ imir axis = {axis}");
            assert!(!f.windows(4).any(|w| w == b"irot"), "mirror only ⇒ no irot");
        }
    }
}
