//! The PNG encoder: a [`PngEncoder`] builder implementing [`gamut_core::EncodeImage`] for each
//! supported pixel layout. This covers the four non-indexed colour types at 8- and 16-bit depth;
//! palette, sub-byte depths, ancillary chunks, and space optimisations layer on in later phases.

use gamut_core::{
    Bilevel, Dimensions, EncodeImage, Error, Gray8, Gray16, GrayAlpha8, GrayAlpha16, ImageRef,
    Indexed8, Pixel, Result, Rgb8, Rgb16, Rgba8, Rgba16,
};
use gamut_deflate::{DeflateEncoder, Level};

use crate::ancillary::{Ancillary, PhysicalUnit, SrgbIntent};
use crate::backend::{IdatDeflater, IdatInfo, Registry, run_deflaters};
use crate::chunk::{self, SIGNATURE};
use crate::color::ColorType;
use crate::filter::{self, FilterStrategy, FilterType};
use crate::palette::PngPalette;
use crate::reduce::{self, Reduced};
use crate::{ihdr, pack};

/// IDAT payload cap. A decoder concatenates consecutive IDATs, so the split is transparent; a
/// large-ish cap keeps the 12-byte per-chunk overhead negligible.
const IDAT_MAX: usize = 1 << 16;

/// Whole-image filter strategies tried by [`FilterStrategy::BruteForce`].
const BRUTE_FORCE_STRATEGIES: [FilterStrategy; 6] = [
    FilterStrategy::None,
    FilterStrategy::Fixed(FilterType::Sub),
    FilterStrategy::Fixed(FilterType::Up),
    FilterStrategy::Fixed(FilterType::Average),
    FilterStrategy::Fixed(FilterType::Paeth),
    FilterStrategy::MinSumAbs,
];

/// A reusable PNG encoder.
#[derive(Debug, Clone)]
pub struct PngEncoder {
    level: Level,
    effort: u8,
    filter: FilterStrategy,
    ancillary: Ancillary,
    auto_reduce: bool,
    clean_transparent: bool,
    backends: Registry<dyn IdatDeflater + Send>,
}

impl Default for PngEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl PngEncoder {
    /// Creates an encoder with balanced [`Level::Default`] compression and the
    /// [`FilterStrategy::MinSumAbs`] filter heuristic.
    #[must_use]
    pub fn new() -> Self {
        Self {
            level: Level::Default,
            effort: DeflateEncoder::DEFAULT_EFFORT,
            filter: FilterStrategy::MinSumAbs,
            ancillary: Ancillary::default(),
            auto_reduce: false,
            clean_transparent: false,
            backends: Registry::default(),
        }
    }

    /// Appends a pluggable [`IdatDeflater`] backend for the IDAT zlib stream (issue #278).
    ///
    /// Backends are tried in **push order**; the built-in [`gamut_deflate`] encoder is the implicit
    /// tail, so pushing nothing keeps today's behaviour byte for byte. A backend that returns
    /// `false` from [`IdatDeflater::supports`] (or [`Error::Unsupported`] from
    /// [`IdatDeflater::deflate`]) is skipped; one that accepts and then fails propagates its error
    /// rather than falling back — see the [`backend`](crate::backend) module docs for the full
    /// contract.
    ///
    /// # Interaction with [`with_compression`](Self::with_compression)
    ///
    /// A pushed deflater **bypasses the configured [`Level`]** (and the
    /// [`with_effort`](Self::with_effort) budget) for every stream it accepts: `Level` is a
    /// `gamut-deflate` concept that a foreign backend knows nothing about, and the seam datum is
    /// only the byte stream. The `Level` still governs the built-in tail, i.e. any stream every
    /// pushed backend declines.
    ///
    /// # Cloning shares backends
    ///
    /// [`PngEncoder`] is [`Clone`], and cloning copies the registry *handles*: the clone and the
    /// original drive the **same** backend instances (they are held behind `Arc<Mutex<…>>`, since
    /// encoding takes `&self`). Push a fresh backend instance if you need independent state.
    ///
    /// Takes `&mut self` and returns `&mut Self` — unlike the `with_*` builder setters — because a
    /// registry accumulates rather than replaces.
    pub fn push_backend(&mut self, backend: impl IdatDeflater + 'static) -> &mut Self {
        self.backends
            .push(std::sync::Arc::new(std::sync::Mutex::new(backend)));
        self
    }

    /// Sets the DEFLATE compression [`Level`] used for the image data. [`Level::Best`] is the
    /// space-efficient (slow) setting.
    #[must_use]
    pub fn with_compression(mut self, level: Level) -> Self {
        self.level = level;
        self
    }

    /// Sets the [`Level::Best`] effort budget: the maximum number of optimal-parse refinement
    /// passes (see [`DeflateEncoder::with_effort`]) applied to every zlib stream this encoder emits
    /// — the IDAT image data and compressed ancillary payloads (`iCCP`, `zTXt`).
    ///
    /// Defaults to [`DeflateEncoder::DEFAULT_EFFORT`]; `0` keeps the lazy seed parse only, and
    /// `zopfli`'s default budget is 15. Ignored at every other [`Level`] and by any pushed
    /// [`IdatDeflater`] backend that accepts a stream.
    #[must_use]
    pub fn with_effort(mut self, effort: u8) -> Self {
        self.effort = effort;
        self
    }

    /// Sets the scanline [`FilterStrategy`].
    #[must_use]
    pub fn with_filter(mut self, filter: FilterStrategy) -> Self {
        self.filter = filter;
        self
    }

    /// Rewrites the colour channels of fully transparent pixels before encoding, so runs of
    /// them compress instead of carrying whatever the source left there.
    ///
    /// Nothing a decoder renders changes -- at `alpha == 0` the colour channels are invisible by
    /// definition -- but the stored samples do, so this is **not** lossless in the strict byte
    /// sense [`with_auto_reduce`](Self::with_auto_reduce) keeps. That is why it is off by
    /// default and separate from it: this crate's other reductions are exactly reversible, and
    /// this one is only reversible in what you can see.
    ///
    /// Worth enabling for sprites, icons and UI assets, where invisible colour noise is common
    /// and can cost real bytes. No effect on an image with no fully transparent pixel, or on a
    /// layout with no alpha channel.
    #[must_use]
    pub fn with_transparent_cleanup(mut self, enabled: bool) -> Self {
        self.clean_transparent = enabled;
        self
    }

    /// Enables automatic lossless reduction of any [`EncodeImage`] input to a smaller encoding
    /// when it does not change any pixel: greyscale (at the smallest exactly-representable bit
    /// depth), palette, alpha-channel drop, and 16→8 demotion when every sample's high and low
    /// bytes agree.
    ///
    /// Off by default so the output colour type and depth match the input. Enable it — ideally
    /// with [`Level::Best`] and [`FilterStrategy::BruteForce`] — for the smallest possible files.
    #[must_use]
    pub fn with_auto_reduce(mut self, enabled: bool) -> Self {
        self.auto_reduce = enabled;
        self
    }

    /// Records an image gamma (gAMA chunk). `gamma` is the encoding gamma, e.g. `1.0 / 2.2`.
    #[must_use]
    pub fn with_gamma(mut self, gamma: f64) -> Self {
        self.ancillary.gamma = Some((gamma * 100_000.0).round().max(0.0) as u32);
        self
    }

    /// Records the standard colour-space rendering intent (sRGB chunk).
    #[must_use]
    pub fn with_srgb(mut self, intent: SrgbIntent) -> Self {
        self.ancillary.set_srgb(intent);
        self
    }

    /// Records the white point and RGB primary chromaticities (cHRM chunk), each as `(x, y)`.
    #[must_use]
    pub fn with_chromaticities(
        mut self,
        white: (f64, f64),
        red: (f64, f64),
        green: (f64, f64),
        blue: (f64, f64),
    ) -> Self {
        let q = |v: f64| (v * 100_000.0).round().max(0.0) as u32;
        self.ancillary.chrm = Some([
            q(white.0),
            q(white.1),
            q(red.0),
            q(red.1),
            q(green.0),
            q(green.1),
            q(blue.0),
            q(blue.1),
        ]);
        self
    }

    /// Records the number of significant bits per channel (sBIT chunk). The length must match the
    /// colour type (1 for grey, 2 for grey+alpha, 3 for RGB/indexed, 4 for RGBA).
    #[must_use]
    pub fn with_significant_bits(mut self, bits: &[u8]) -> Self {
        self.ancillary.sbit = Some(bits.to_vec());
        self
    }

    /// Records a greyscale background colour (bKGD chunk) for greyscale images.
    #[must_use]
    pub fn with_background_gray(mut self, gray: u16) -> Self {
        self.ancillary.bkgd = Some(gray.to_be_bytes().to_vec());
        self
    }

    /// Records an RGB background colour (bKGD chunk) for truecolour images.
    #[must_use]
    pub fn with_background_rgb(mut self, red: u16, green: u16, blue: u16) -> Self {
        let mut data = Vec::with_capacity(6);
        data.extend_from_slice(&red.to_be_bytes());
        data.extend_from_slice(&green.to_be_bytes());
        data.extend_from_slice(&blue.to_be_bytes());
        self.ancillary.bkgd = Some(data);
        self
    }

    /// Records a palette-index background colour (bKGD chunk) for indexed images.
    #[must_use]
    pub fn with_background_index(mut self, index: u8) -> Self {
        self.ancillary.bkgd = Some(vec![index]);
        self
    }

    /// Records the intended physical pixel dimensions (pHYs chunk).
    #[must_use]
    pub fn with_physical_dimensions(mut self, x_ppu: u32, y_ppu: u32, unit: PhysicalUnit) -> Self {
        self.ancillary.set_physical(x_ppu, y_ppu, unit);
        self
    }

    /// Records the last-modification time (tIME chunk), in UTC.
    #[must_use]
    pub fn with_time(
        mut self,
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    ) -> Self {
        self.ancillary
            .set_time(year, month, day, hour, minute, second);
        self
    }

    /// Adds an uncompressed Latin-1 text annotation (tEXt chunk).
    #[must_use]
    pub fn with_text(mut self, keyword: &str, text: &str) -> Self {
        self.ancillary.add_text_latin1(keyword, text);
        self
    }

    /// Adds a zlib-compressed Latin-1 text annotation (zTXt chunk).
    #[must_use]
    pub fn with_compressed_text(mut self, keyword: &str, text: &str) -> Self {
        self.ancillary.add_text_compressed(keyword, text);
        self
    }

    /// Adds an uncompressed UTF-8 text annotation (iTXt chunk).
    #[must_use]
    pub fn with_international_text(mut self, keyword: &str, text: &str) -> Self {
        self.ancillary.add_text_international(keyword, text);
        self
    }

    /// Embeds raw EXIF metadata (eXIf chunk). `exif` is the EXIF/TIFF byte stream beginning with the
    /// byte-order marker (`II`/`MM`) — for example the bytes produced by `gamut-exif`.
    #[must_use]
    pub fn with_exif(mut self, exif: &[u8]) -> Self {
        self.ancillary.exif = Some(exif.to_vec());
        self
    }

    /// Embeds an ICC colour profile (iCCP chunk), zlib-compressed. `profile` is the raw ICC profile
    /// — for example the bytes produced by `gamut-icc`. (Mutually exclusive with [`Self::with_srgb`]
    /// per the spec; set only one.)
    #[must_use]
    pub fn with_icc_profile(mut self, name: &str, profile: &[u8]) -> Self {
        self.ancillary.iccp = Some((name.to_string(), profile.to_vec()));
        self
    }

    /// Embeds an XMP packet (an iTXt chunk with the standard `XML:com.adobe.xmp` keyword). `xmp` is
    /// the XMP/RDF document — for example the bytes produced by `gamut-xmp`.
    #[must_use]
    pub fn with_xmp(mut self, xmp: &str) -> Self {
        self.ancillary
            .add_text_international("XML:com.adobe.xmp", xmp);
        self
    }

    /// Encodes an 8-bit indexed (palette) image. Indexed colour does not fit the single-buffer
    /// [`EncodeImage`] shape because it needs a separate palette, so it is an inherent method.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if any index is out of range for `palette`.
    pub fn encode_indexed8(
        &self,
        image: ImageRef<'_, Indexed8>,
        palette: &PngPalette,
        out: &mut Vec<u8>,
    ) -> Result<usize> {
        let indices = image.as_samples();
        let max_index = indices.iter().copied().max().unwrap_or(0);
        if usize::from(max_index) >= palette.len() {
            return Err(Error::invalid_input(
                env!("CARGO_PKG_NAME"),
                "PNG: palette index out of range",
            ));
        }
        let dims = image.dimensions();
        // Use the smallest bit depth that holds every index — a free, lossless space win.
        let depth = reduce::index_bit_depth(palette.len());
        let packed;
        let sample_bytes = if depth < 8 {
            packed =
                pack::pack_scanlines(indices, dims.width as usize, dims.height as usize, depth);
            packed.as_slice()
        } else {
            indices
        };
        let plte = palette.plte();
        let trns = palette.trns();
        self.write_png(
            (dims.width, dims.height),
            sample_bytes,
            ColorType::Indexed,
            depth,
            |out| {
                chunk::write_chunk(out, *b"PLTE", &plte);
                if let Some(alpha) = trns {
                    chunk::write_chunk(out, *b"tRNS", alpha);
                }
            },
            out,
        )
    }

    /// Encodes an 8-bit-per-sample image (samples are already PNG's storage bytes).
    fn encode_8bit<P: Pixel<Sample = u8>>(
        &self,
        image: ImageRef<'_, P>,
        color: ColorType,
        out: &mut Vec<u8>,
    ) -> Result<usize> {
        let dims = image.dimensions();
        self.write_png(
            (dims.width, dims.height),
            image.as_samples(),
            color,
            8,
            |_| {},
            out,
        )
    }

    /// The cleaned samples, or `None` to use the caller's buffer unchanged — either because the
    /// knob is off or because the image has no fully transparent pixel.
    fn cleaned_samples(&self, samples: &[u8], channels: usize) -> Option<Vec<u8>> {
        self.clean_transparent
            .then(|| reduce::clean_transparent(samples, channels))
            .flatten()
    }

    /// Encodes a 16-bit-per-sample image, serialising samples big-endian (PNG's network byte order).
    fn encode_16bit<P: Pixel<Sample = u16>>(
        &self,
        image: ImageRef<'_, P>,
        color: ColorType,
        out: &mut Vec<u8>,
    ) -> Result<usize> {
        let dims = image.dimensions();
        let samples = image.as_samples();
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for &sample in samples {
            bytes.extend_from_slice(&sample.to_be_bytes());
        }
        self.write_png((dims.width, dims.height), &bytes, color, 16, |_| {}, out)
    }

    /// Shared back end: signature → IHDR → `pre_idat` chunks (e.g. PLTE/tRNS) → filtered +
    /// DEFLATE-compressed scanlines as IDAT(s) → IEND. `sample_bytes` is the image in PNG storage
    /// order; the stride is derived from `color` and `bit_depth`.
    fn write_png<F: FnOnce(&mut Vec<u8>)>(
        &self,
        (width, height): (u32, u32),
        sample_bytes: &[u8],
        color: ColorType,
        bit_depth: u8,
        pre_idat: F,
        out: &mut Vec<u8>,
    ) -> Result<usize> {
        // Stride in bytes per pixel (≥1, even for sub-byte depths) and the padded row length.
        let bits_per_pixel = color.channels() * bit_depth as usize;
        let bpp = bits_per_pixel.div_ceil(8).max(1);
        let row_bytes = (width as usize * bits_per_pixel).div_ceil(8);

        let start = out.len();
        out.extend_from_slice(&SIGNATURE);
        ihdr::write(out, width, height, bit_depth, color);
        self.ancillary.write_pre_plte(out, self.effort); // colour-space chunks precede PLTE
        pre_idat(out); // PLTE + tRNS (indexed only)
        self.ancillary.write_post_plte(out, self.effort); // background / physical / timing / text

        let idat = self.compress_scanlines(
            sample_bytes,
            row_bytes,
            bpp,
            IdatInfo::new(
                width,
                height,
                bit_depth,
                color,
                // The filtered stream is one filter-type byte plus `row_bytes` per scanline; the
                // encoder never interlaces, so this is exact.
                (height as usize) * (row_bytes + 1),
            ),
        )?;
        write_idat(out, &idat);

        chunk::write_chunk(out, *b"IEND", &[]);
        Ok(out.len() - start)
    }

    /// Filters and DEFLATE-compresses the scanlines into a zlib stream. For
    /// [`FilterStrategy::BruteForce`] it compresses under every whole-image strategy and keeps the
    /// smallest; otherwise it uses the single configured strategy.
    fn compress_scanlines(
        &self,
        sample_bytes: &[u8],
        row_bytes: usize,
        bpp: usize,
        info: IdatInfo,
    ) -> Result<Vec<u8>> {
        let deflate = DeflateEncoder::new()
            .with_level(self.level)
            .with_effort(self.effort);
        // Every candidate stream goes through the same seam: a pushed backend that accepts sees
        // each brute-force candidate, and the smallest result still wins.
        let compress = |strategy| {
            let filtered = filter::filter_image(strategy, sample_bytes, row_bytes, bpp);
            run_deflaters(&self.backends, &info, &filtered, |raw| {
                let mut idat = Vec::new();
                deflate.zlib_compress(raw, &mut idat);
                idat
            })
        };
        if matches!(self.filter, FilterStrategy::BruteForce) {
            // `min_by_key` keeps the *first* minimum, so a tie resolves to the earlier (more
            // preferred) strategy — the behaviour the golden outputs are pinned to.
            let candidates = BRUTE_FORCE_STRATEGIES
                .into_iter()
                .map(compress)
                .collect::<Result<Vec<_>>>()?;
            Ok(candidates
                .into_iter()
                .min_by_key(Vec::len)
                .unwrap_or_default())
        } else {
            compress(self.filter)
        }
    }

    /// Writes `reduced`, unless it is a palette encoding that turns out *larger* than encoding
    /// the image untouched — in which case the untouched one wins.
    ///
    /// [`reduce::analyze8`] chooses by comparing **raw** sizes, and raw size does not predict
    /// compressed size when one candidate's bytes are incompressible and the other's are not. A
    /// palette carries a `PLTE` (and often `tRNS`) chunk that DEFLATE cannot touch, while the
    /// pixels it replaces may compress by two orders of magnitude. On a 128x128 image with 64
    /// colours the estimate sees 16 664 bytes against 65 536 and picks the palette by 4x — and
    /// the finished file is 451 bytes against 405. The crossover sits near 160x160, so the
    /// estimate is right on large images and wrong on small ones.
    ///
    /// Rather than guess a correction factor, the two candidates are encoded and the smaller
    /// kept. That is exactly what [`FilterStrategy::BruteForce`] already does for filters, it
    /// needs no tuned constant, and it cannot be worse than either candidate alone. A tie keeps
    /// the palette, which decodes with less work.
    ///
    /// Only the reductions that *carry a chunk* pay for the second encode — a palette's `PLTE`
    /// (+ `tRNS`), or a colour key's `tRNS`. Greyscale, alpha-drop and 16→8 demotion add no chunks
    /// at all, so for them the raw comparison is sound and this returns immediately.
    fn write_reduced_or_native(
        &self,
        dims: Dimensions,
        reduced: Reduced,
        native: impl FnOnce(&mut Vec<u8>) -> Result<usize>,
        out: &mut Vec<u8>,
    ) -> Result<usize> {
        let carries_chunks = matches!(
            reduced,
            Reduced::Indexed { .. } | Reduced::Rgb8Keyed { .. } | Reduced::GrayKeyed { .. }
        );
        if !carries_chunks {
            return self.write_reduced(dims, reduced, out);
        }
        let mut palette_encoding = Vec::new();
        self.write_reduced(dims, reduced, &mut palette_encoding)?;
        let mut native_encoding = Vec::new();
        native(&mut native_encoding)?;

        let winner = if prefers_native(native_encoding.len(), palette_encoding.len()) {
            native_encoding
        } else {
            palette_encoding
        };
        out.extend_from_slice(&winner);
        Ok(winner.len())
    }

    /// Writes a reduced encoding chosen by [`reduce::analyze8`] / [`reduce::analyze16`].
    fn write_reduced(
        &self,
        dims: Dimensions,
        reduced: Reduced,
        out: &mut Vec<u8>,
    ) -> Result<usize> {
        let wh = (dims.width, dims.height);
        match reduced {
            Reduced::Gray { depth, samples } => {
                let packed;
                let sample_bytes = if depth < 8 {
                    packed = pack::pack_scanlines(
                        &samples,
                        dims.width as usize,
                        dims.height as usize,
                        depth,
                    );
                    packed.as_slice()
                } else {
                    &samples
                };
                self.write_png(wh, sample_bytes, ColorType::Grayscale, depth, |_| {}, out)
            }
            Reduced::GrayAlpha8(samples) => {
                self.write_png(wh, &samples, ColorType::GrayscaleAlpha, 8, |_| {}, out)
            }
            Reduced::Rgb8(samples) => {
                self.write_png(wh, &samples, ColorType::Truecolor, 8, |_| {}, out)
            }
            // §11.3.2.1: for truecolour, tRNS is three 16-bit big-endian samples naming the one
            // colour a decoder renders as fully transparent. At depth 8 the high byte is zero.
            Reduced::Rgb8Keyed { samples, key } => self.write_png(
                wh,
                &samples,
                ColorType::Truecolor,
                8,
                |out| {
                    let trns = [0, key[0], 0, key[1], 0, key[2]];
                    chunk::write_chunk(out, *b"tRNS", &trns);
                },
                out,
            ),
            // ...and for greyscale, one 16-bit big-endian sample.
            Reduced::GrayKeyed { samples, key } => self.write_png(
                wh,
                &samples,
                ColorType::Grayscale,
                8,
                |out| chunk::write_chunk(out, *b"tRNS", &[0, key]),
                out,
            ),
            Reduced::Rgba8(samples) => {
                self.write_png(wh, &samples, ColorType::TruecolorAlpha, 8, |_| {}, out)
            }
            Reduced::Gray16Be(bytes) => {
                self.write_png(wh, &bytes, ColorType::Grayscale, 16, |_| {}, out)
            }
            Reduced::GrayAlpha16Be(bytes) => {
                self.write_png(wh, &bytes, ColorType::GrayscaleAlpha, 16, |_| {}, out)
            }
            Reduced::Rgb16Be(bytes) => {
                self.write_png(wh, &bytes, ColorType::Truecolor, 16, |_| {}, out)
            }
            Reduced::Indexed {
                depth,
                indices,
                plte,
                trns,
            } => {
                let packed;
                let sample_bytes = if depth < 8 {
                    packed = pack::pack_scanlines(
                        &indices,
                        dims.width as usize,
                        dims.height as usize,
                        depth,
                    );
                    packed.as_slice()
                } else {
                    &indices
                };
                self.write_png(
                    wh,
                    sample_bytes,
                    ColorType::Indexed,
                    depth,
                    |out| {
                        chunk::write_chunk(out, *b"PLTE", &plte);
                        if let Some(alpha) = &trns {
                            chunk::write_chunk(out, *b"tRNS", alpha);
                        }
                    },
                    out,
                )
            }
        }
    }
}

/// Whether the unreduced encoding beats the palette one, for [`PngEncoder::write_reduced_or_native`].
///
/// **A tie keeps the palette**, which decodes with less work for the same bytes. Split out because
/// engineering two encodings of the same image to land on exactly equal lengths is not something a
/// fixture can do reliably, so the tie is only assertable here.
fn prefers_native(native_len: usize, palette_len: usize) -> bool {
    native_len < palette_len
}

/// Writes the zlib datastream as one or more consecutive IDAT chunks.
fn write_idat(out: &mut Vec<u8>, zlib_stream: &[u8]) {
    if zlib_stream.is_empty() {
        chunk::write_chunk(out, *b"IDAT", &[]);
        return;
    }
    for piece in zlib_stream.chunks(IDAT_MAX) {
        chunk::write_chunk(out, *b"IDAT", piece);
    }
}

// One impl per supported pixel layout. Indexed colour is handled separately (it needs a palette);
// CMYK has no PNG colour type.
impl EncodeImage<Gray8> for PngEncoder {
    fn encode_image(&self, image: ImageRef<'_, Gray8>, out: &mut Vec<u8>) -> Result<usize> {
        if self.auto_reduce
            && let Some(reduced) = reduce::analyze8(image.as_samples(), 1)
        {
            return self.write_reduced_or_native(
                image.dimensions(),
                reduced,
                |o| self.encode_8bit(image, ColorType::Grayscale, o),
                out,
            );
        }
        self.encode_8bit(image, ColorType::Grayscale, out)
    }
}
impl EncodeImage<Bilevel> for PngEncoder {
    /// Bilevel pixels (0 = black, non-zero = white) are packed to a 1-bit greyscale image.
    fn encode_image(&self, image: ImageRef<'_, Bilevel>, out: &mut Vec<u8>) -> Result<usize> {
        let dims = image.dimensions();
        let bits: Vec<u8> = image
            .as_samples()
            .iter()
            .map(|&v| u8::from(v != 0))
            .collect();
        let packed = pack::pack_scanlines(&bits, dims.width as usize, dims.height as usize, 1);
        self.write_png(
            (dims.width, dims.height),
            &packed,
            ColorType::Grayscale,
            1,
            |_| {},
            out,
        )
    }
}
impl EncodeImage<Rgb8> for PngEncoder {
    fn encode_image(&self, image: ImageRef<'_, Rgb8>, out: &mut Vec<u8>) -> Result<usize> {
        if self.auto_reduce
            && let Some(reduced) = reduce::analyze8(image.as_samples(), 3)
        {
            return self.write_reduced_or_native(
                image.dimensions(),
                reduced,
                |o| self.encode_8bit(image, ColorType::Truecolor, o),
                out,
            );
        }
        self.encode_8bit(image, ColorType::Truecolor, out)
    }
}
impl EncodeImage<Rgba8> for PngEncoder {
    fn encode_image(&self, image: ImageRef<'_, Rgba8>, out: &mut Vec<u8>) -> Result<usize> {
        let cleaned = self.cleaned_samples(image.as_samples(), 4);
        let samples = cleaned.as_deref().unwrap_or_else(|| image.as_samples());
        let dims = image.dimensions();
        if self.auto_reduce
            && let Some(reduced) = reduce::analyze8(samples, 4)
        {
            return self.write_reduced_or_native(
                dims,
                reduced,
                |o| {
                    self.write_png(
                        (dims.width, dims.height),
                        samples,
                        ColorType::TruecolorAlpha,
                        8,
                        |_| {},
                        o,
                    )
                },
                out,
            );
        }
        self.write_png(
            (dims.width, dims.height),
            samples,
            ColorType::TruecolorAlpha,
            8,
            |_| {},
            out,
        )
    }
}
impl EncodeImage<GrayAlpha8> for PngEncoder {
    fn encode_image(&self, image: ImageRef<'_, GrayAlpha8>, out: &mut Vec<u8>) -> Result<usize> {
        let cleaned = self.cleaned_samples(image.as_samples(), 2);
        let samples = cleaned.as_deref().unwrap_or_else(|| image.as_samples());
        let dims = image.dimensions();
        if self.auto_reduce
            && let Some(reduced) = reduce::analyze8(samples, 2)
        {
            return self.write_reduced_or_native(
                dims,
                reduced,
                |o| {
                    self.write_png(
                        (dims.width, dims.height),
                        samples,
                        ColorType::GrayscaleAlpha,
                        8,
                        |_| {},
                        o,
                    )
                },
                out,
            );
        }
        self.write_png(
            (dims.width, dims.height),
            samples,
            ColorType::GrayscaleAlpha,
            8,
            |_| {},
            out,
        )
    }
}
impl EncodeImage<Gray16> for PngEncoder {
    fn encode_image(&self, image: ImageRef<'_, Gray16>, out: &mut Vec<u8>) -> Result<usize> {
        if self.auto_reduce
            && let Some(reduced) = reduce::analyze16(image.as_samples(), 1)
        {
            return self.write_reduced_or_native(
                image.dimensions(),
                reduced,
                |o| self.encode_16bit(image, ColorType::Grayscale, o),
                out,
            );
        }
        self.encode_16bit(image, ColorType::Grayscale, out)
    }
}
impl EncodeImage<Rgb16> for PngEncoder {
    fn encode_image(&self, image: ImageRef<'_, Rgb16>, out: &mut Vec<u8>) -> Result<usize> {
        if self.auto_reduce
            && let Some(reduced) = reduce::analyze16(image.as_samples(), 3)
        {
            return self.write_reduced_or_native(
                image.dimensions(),
                reduced,
                |o| self.encode_16bit(image, ColorType::Truecolor, o),
                out,
            );
        }
        self.encode_16bit(image, ColorType::Truecolor, out)
    }
}
impl EncodeImage<Rgba16> for PngEncoder {
    fn encode_image(&self, image: ImageRef<'_, Rgba16>, out: &mut Vec<u8>) -> Result<usize> {
        if self.auto_reduce
            && let Some(reduced) = reduce::analyze16(image.as_samples(), 4)
        {
            return self.write_reduced_or_native(
                image.dimensions(),
                reduced,
                |o| self.encode_16bit(image, ColorType::TruecolorAlpha, o),
                out,
            );
        }
        self.encode_16bit(image, ColorType::TruecolorAlpha, out)
    }
}
impl EncodeImage<GrayAlpha16> for PngEncoder {
    fn encode_image(&self, image: ImageRef<'_, GrayAlpha16>, out: &mut Vec<u8>) -> Result<usize> {
        if self.auto_reduce
            && let Some(reduced) = reduce::analyze16(image.as_samples(), 2)
        {
            return self.write_reduced_or_native(
                image.dimensions(),
                reduced,
                |o| self.encode_16bit(image, ColorType::GrayscaleAlpha, o),
                out,
            );
        }
        self.encode_16bit(image, ColorType::GrayscaleAlpha, out)
    }
}

#[cfg(test)]
mod tests {
    use gamut_core::Dimensions;

    use super::*;

    #[test]
    fn emits_signature_ihdr_idat_iend() {
        let src = vec![0u8; 2 * 2 * 3];
        let img = ImageRef::<Rgb8>::new(&src, Dimensions::new(2, 2).unwrap()).unwrap();
        let mut png = Vec::new();
        PngEncoder::new().encode_image(img, &mut png).unwrap();
        assert_eq!(&png[..8], &SIGNATURE);
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
    }

    #[test]
    fn ihdr_reports_color_type_and_depth() {
        // A 16-bit grayscale-alpha image should declare colour type 4, bit depth 16.
        let src = vec![0u16; 3 * 3 * 2];
        let img = ImageRef::<GrayAlpha16>::new(&src, Dimensions::new(3, 3).unwrap()).unwrap();
        let mut png = Vec::new();
        PngEncoder::new().encode_image(img, &mut png).unwrap();
        // IHDR data starts at byte 16: width(4) height(4) depth(1) colortype(1).
        assert_eq!(png[24], 16, "bit depth");
        assert_eq!(png[25], ColorType::GrayscaleAlpha.code(), "colour type");
    }

    #[test]
    fn returns_the_number_of_bytes_appended_not_the_total_length() {
        // `write_png` reports `out.len() - start`; encoding into a non-empty buffer is what tells
        // that apart from the buffer's total length.
        let src = vec![0u8; 2 * 2 * 3];
        let img = ImageRef::<Rgb8>::new(&src, Dimensions::new(2, 2).unwrap()).unwrap();
        let mut fresh = Vec::new();
        let alone = PngEncoder::new().encode_image(img, &mut fresh).unwrap();
        assert_eq!(alone, fresh.len());

        let mut appended = vec![0xAAu8; 17];
        let written = PngEncoder::new().encode_image(img, &mut appended).unwrap();
        assert_eq!(written, alone, "only the PNG's own bytes are counted");
        assert_eq!(appended.len(), 17 + alone);
        assert_eq!(&appended[17..], &fresh[..], "the prefix is left untouched");
    }

    #[test]
    fn a_tie_between_palette_and_native_keeps_the_palette() {
        assert!(prefers_native(10, 11), "smaller native wins");
        assert!(!prefers_native(11, 10), "smaller palette wins");
        assert!(!prefers_native(10, 10), "a tie keeps the palette");
    }

    #[test]
    fn brute_force_keeps_the_first_strategy_on_a_tie() {
        // A 1x1 image compresses to the same length under every strategy, so the tie-break is what
        // picks the output. `BRUTE_FORCE_STRATEGIES` is in preference order and the first minimum
        // wins, so the filter byte must be `None` (0) — not the last strategy's choice.
        let src = vec![200u8];
        let img = ImageRef::<Gray8>::new(&src, Dimensions::new(1, 1).unwrap()).unwrap();
        let mut brute = Vec::new();
        PngEncoder::new()
            .with_filter(FilterStrategy::BruteForce)
            .encode_image(img, &mut brute)
            .unwrap();
        let mut none = Vec::new();
        PngEncoder::new()
            .with_filter(FilterStrategy::None)
            .encode_image(img, &mut none)
            .unwrap();
        assert_eq!(
            brute, none,
            "the tie must resolve to the first (None) candidate"
        );
        // And it is genuinely a tie the later strategies could have won.
        let mut paeth = Vec::new();
        PngEncoder::new()
            .with_filter(FilterStrategy::Fixed(FilterType::Paeth))
            .encode_image(img, &mut paeth)
            .unwrap();
        assert_eq!(brute.len(), paeth.len(), "same length, different bytes");
        assert_ne!(brute, paeth);
    }

    #[test]
    fn large_stream_splits_into_multiple_idats() {
        // Incompressible data larger than IDAT_MAX must yield more than one IDAT chunk.
        let mut out = Vec::new();
        let big = vec![0xABu8; IDAT_MAX * 2 + 100];
        write_idat(&mut out, &big);
        let idats = out.windows(4).filter(|w| *w == b"IDAT").count();
        assert!(idats >= 3, "expected multiple IDAT chunks, found {idats}");
    }
}
