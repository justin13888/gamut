//! The [`JxlDecoder`]: a typed front end over a stack of JPEG XL codestream decoders — any backend
//! pushed with [`JxlDecoder::push_backend`], and, last, the built-in pure-Rust jxl-rs wrapper
//! ([`crate::jxlrs`]).

use gamut_core::{
    DecodeImage, Error, Gray8, Gray16, GrayAlpha8, GrayAlpha16, ImageBuf, Pixel, PixelFormat,
    Result, Rgb8, Rgb16, Rgba8, Rgba16,
};

use crate::backend::{
    JxlCodestreamDecoder, JxlDecoded, JxlFraming, JxlOwnedSamples, JxlStreamInfo, Registry,
};
#[cfg(feature = "decode")]
pub use crate::jxlrs::JxlInfo;

/// The refusal returned when no backend can decode: nothing was pushed, and the built-in jxl-rs
/// tail is not compiled into this build (no `decode` feature).
///
/// Always compiled (and unit-tested) even where the tail *is* present, so its message stays pinned
/// on every build rather than only on the targets that can return it.
#[cfg_attr(feature = "decode", allow(dead_code))]
fn no_decode_backend() -> Error {
    Error::unsupported(
        env!("CARGO_PKG_NAME"),
        "JXL: no decode backend (enable the `decode` feature or push a codestream backend)",
    )
}

/// The error for a backend that returned a raster in a layout other than the one requested.
fn wrong_backend_layout() -> Error {
    Error::invalid_input(
        env!("CARGO_PKG_NAME"),
        "JXL: backend returned a raster in the wrong pixel layout",
    )
}

/// A JPEG XL decoder.
///
/// Decodes both JPEG XL framings — a bare codestream and the ISO BMFF `.jxl` container — into any of
/// the eight supported pixel layouts (8/16-bit grayscale, gray+alpha, RGB, RGBA) through the
/// [`DecodeImage`](gamut_core::DecodeImage) trait. Where the requested layout and the stream differ,
/// the built-in decoder converts internally: grayscale expands to RGB, a missing alpha channel is
/// padded opaque, and a present-but-unwanted alpha is dropped. It deliberately refuses to *guess*: a
/// colour image cannot be decoded as grayscale, animated input is rejected, and premultiplied
/// (associated) alpha is rejected — each an [`Error::Unsupported`] that a later version may relax.
///
/// Construct it with [`JxlDecoder::new`] or [`Default`], then optionally set
/// [`JxlDecoder::with_codestream_bit_depth`].
///
/// # Backends
///
/// The codestream itself is decoded by a [`JxlCodestreamDecoder`]. With the `decode` feature the
/// pure-Rust jxl-rs wrapper is the implicit **last** backend, so the default decoder needs no
/// wiring. [`JxlDecoder::push_backend`] inserts a platform or alternate decoder *ahead* of it; with
/// neither a pushed backend nor the built-in tail, decoding reports [`Error::Unsupported`]. See
/// [`crate::backend`] for the fallback contract.
///
/// The type is `Clone` but **not `Copy`** (it was `Copy` before backends existed): it owns a shared
/// backend registry. Cloning shares that registry — a backend pushed onto a clone is visible through
/// the original. `PartialEq`/`Eq` compare the **configuration** only, since backends are opaque.
#[derive(Debug, Clone, Default)]
pub struct JxlDecoder {
    /// Whether integer output carries the codestream's declared bit depth.
    codestream_bit_depth: bool,
    /// Pushed codestream backends, tried in push order ahead of the built-in jxl-rs tail.
    backends: Registry<dyn JxlCodestreamDecoder>,
}

impl PartialEq for JxlDecoder {
    /// Compares the decoder **configuration**; the backend registries are ignored, since a
    /// [`JxlCodestreamDecoder`] is an opaque trait object with no notion of equality.
    fn eq(&self, other: &Self) -> bool {
        self.codestream_bit_depth == other.codestream_bit_depth
    }
}

impl Eq for JxlDecoder {}

impl JxlDecoder {
    /// Creates a decoder with the default configuration.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Selects whether integer output carries the **codestream's declared bit depth** instead of
    /// the output type's full range. Off by default.
    ///
    /// A JPEG XL stream declares its samples' bit depth N (e.g. 10-bit). By default a
    /// 16-bit decode scales samples to full-range `0 ..= 65535`; with this set, samples keep
    /// their coded range `0 ..= 2^N - 1` — the reading a raw-code-value consumer (e.g. an N-bit
    /// DNG tile) needs. Streams with a float sample type are unaffected. Returns the updated
    /// decoder for chaining.
    #[must_use]
    pub fn with_codestream_bit_depth(mut self, enabled: bool) -> Self {
        self.codestream_bit_depth = enabled;
        self
    }

    /// Whether integer output carries the codestream's declared bit depth (see
    /// [`JxlDecoder::with_codestream_bit_depth`]).
    #[must_use]
    pub fn codestream_bit_depth(&self) -> bool {
        self.codestream_bit_depth
    }

    /// Pushes a [`JxlCodestreamDecoder`] onto the end of this decoder's backend list, ahead of the
    /// built-in jxl-rs tail. Returns `&mut self` for chaining.
    ///
    /// Backends are tried in push order; the first whose
    /// [`supports`](JxlCodestreamDecoder::supports) returns `true` produces the raster, and the
    /// built-in wrapper (when compiled in) is tried last. A backend that accepts a stream and then
    /// fails propagates its error rather than falling through — see [`crate::backend`] for the full
    /// contract.
    ///
    /// A backend must return **exactly** the layout
    /// [`JxlStreamInfo::format`](crate::JxlStreamInfo::format) asks for; the host does not reshape
    /// its output, and a mismatch is a typed error rather than a silent conversion.
    pub fn push_backend(&mut self, backend: impl JxlCodestreamDecoder + 'static) -> &mut Self {
        self.backends.push(Box::new(backend));
        self
    }

    /// Parses the stream's headers and returns its basic properties without decoding any pixels.
    ///
    /// Always uses the built-in jxl-rs header parser — a pushed backend is not consulted, so the
    /// answer is the crate's own reading of the stream.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if the data is not a decodable JPEG XL stream or is
    /// truncated before the image headers.
    #[cfg(feature = "decode")]
    pub fn info(&self, data: &[u8]) -> Result<JxlInfo> {
        crate::jxlrs::info(data)
    }

    /// Returns the ICC profile **embedded** in the stream's metadata, or `None` when the stream
    /// signals its colour as a structured (enumerated) encoding — sRGB, PQ, HLG, and friends —
    /// instead of carrying profile bytes.
    ///
    /// Only the stream's headers are parsed; no pixels are decoded. The returned bytes are exactly
    /// the attached profile (what [`crate::ColorSpec::Icc`] set at encode time, when the stream was
    /// produced by gamut). This is a metadata accessor: the pixel-decoding paths still return
    /// samples in the stream's own colour encoding, without applying any ICC transform.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if the data is not a decodable JPEG XL stream or is
    /// truncated before the colour metadata.
    #[cfg(feature = "decode")]
    pub fn embedded_icc_profile(&self, data: &[u8]) -> Result<Option<Vec<u8>>> {
        crate::jxlrs::embedded_icc_profile(data)
    }

    /// The stream's dimensions when the built-in header parser can determine them, else `None`.
    ///
    /// Used only to populate [`JxlStreamInfo::dimensions`](crate::JxlStreamInfo::dimensions), and
    /// only when a backend has actually been pushed, so the default decode path never pays for it.
    fn probe_dimensions(&self, data: &[u8]) -> Option<gamut_core::Dimensions> {
        #[cfg(feature = "decode")]
        {
            crate::jxlrs::info(data).ok().map(|info| info.dimensions)
        }
        #[cfg(not(feature = "decode"))]
        {
            let _ = data;
            None
        }
    }

    /// Runs the pushed backends over `data` in push order, returning the first accepted raster or
    /// `None` when every backend declined (so the caller falls through to the built-in tail).
    fn dispatch_backends(&self, data: &[u8], format: PixelFormat) -> Result<Option<JxlDecoded>> {
        if self.backends.is_empty() {
            return Ok(None);
        }
        let info = JxlStreamInfo::new(
            format,
            JxlFraming::detect(data),
            self.probe_dimensions(data),
            self.codestream_bit_depth,
        );
        let mut backends = self.backends.lock();
        for backend in backends.iter_mut() {
            if !backend.supports(&info) {
                continue;
            }
            match backend.decode(&info, data) {
                Ok(decoded) => {
                    if decoded.format() != format {
                        return Err(wrong_backend_layout());
                    }
                    return Ok(Some(decoded));
                }
                // A late decline: fall through exactly as `supports() == false` would.
                Err(error) if error.kind() == gamut_core::ErrorKind::Unsupported => continue,
                // Terminal: the backend accepted the stream and failed, so propagate.
                Err(error) => return Err(error),
            }
        }
        Ok(None)
    }
}

/// Implements [`DecodeImage`] for each supported layout: the pushed backends first, then the
/// built-in jxl-rs tail (or a typed refusal where it is not compiled in). The macro names only the
/// owned-sample variant a layout's storage width implies; every other layout fact comes from
/// [`Pixel::FORMAT`] via the crate's single layout table.
macro_rules! impl_decode_image {
    ($($pixel:ty => $variant:ident;)*) => {$(
        impl DecodeImage<$pixel> for JxlDecoder {
            fn decode_image(&self, data: &[u8]) -> Result<ImageBuf<$pixel>> {
                if let Some(decoded) =
                    self.dispatch_backends(data, <$pixel as Pixel>::FORMAT)?
                {
                    let dimensions = decoded.dimensions();
                    return match decoded.into_samples() {
                        JxlOwnedSamples::$variant(samples) => {
                            ImageBuf::<$pixel>::new(samples, dimensions)
                        }
                        // Unreachable in practice: `JxlDecoded::new` already ties the sample
                        // variant to the format, and the format was checked above.
                        _ => Err(wrong_backend_layout()),
                    };
                }
                #[cfg(feature = "decode")]
                {
                    crate::jxlrs::decode_to_buf::<$pixel>(data, self.codestream_bit_depth)
                }
                #[cfg(not(feature = "decode"))]
                {
                    Err(no_decode_backend())
                }
            }

            fn decode_image_into(&self, data: &[u8], dst: &mut ImageBuf<$pixel>) -> Result<()> {
                if let Some(decoded) =
                    self.dispatch_backends(data, <$pixel as Pixel>::FORMAT)?
                {
                    let dimensions = decoded.dimensions();
                    return match decoded.into_samples() {
                        JxlOwnedSamples::$variant(samples) => {
                            *dst = ImageBuf::<$pixel>::new(samples, dimensions)?;
                            Ok(())
                        }
                        _ => Err(wrong_backend_layout()),
                    };
                }
                #[cfg(feature = "decode")]
                {
                    crate::jxlrs::decode_into_buf::<$pixel>(data, self.codestream_bit_depth, dst)
                }
                #[cfg(not(feature = "decode"))]
                {
                    let _ = dst;
                    Err(no_decode_backend())
                }
            }
        }
    )*};
}

impl_decode_image! {
    Gray8       => U8;
    GrayAlpha8  => U8;
    Rgb8        => U8;
    Rgba8       => U8;
    Gray16      => U16;
    GrayAlpha16 => U16;
    Rgb16       => U16;
    Rgba16      => U16;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use gamut_core::Dimensions;

    use super::*;

    #[test]
    fn new_and_default_are_equal_and_configuration_compares() {
        assert_eq!(JxlDecoder::new(), JxlDecoder::default());
        assert!(!JxlDecoder::new().codestream_bit_depth());
        assert!(
            JxlDecoder::new()
                .with_codestream_bit_depth(true)
                .codestream_bit_depth()
        );
        assert_ne!(
            JxlDecoder::new(),
            JxlDecoder::new().with_codestream_bit_depth(true)
        );
    }

    #[test]
    fn the_refusal_errors_are_pinned() {
        let wrong = wrong_backend_layout();
        assert_eq!(wrong.kind(), gamut_core::ErrorKind::InvalidInput);
        assert_eq!(
            wrong.static_message(),
            Some("JXL: backend returned a raster in the wrong pixel layout")
        );
        let missing = no_decode_backend();
        assert_eq!(missing.kind(), gamut_core::ErrorKind::Unsupported);
        assert_eq!(
            missing.static_message(),
            Some(
                "JXL: no decode backend (enable the `decode` feature or push a codestream backend)"
            )
        );
    }

    /// A backend answering `supports` from a flag and `decode` with a canned outcome, counting both.
    struct FixedBackend {
        supported: bool,
        outcome: Result<JxlDecoded>,
        supports_calls: Arc<AtomicUsize>,
        decode_calls: Arc<AtomicUsize>,
        /// The info the last call saw.
        seen: Arc<std::sync::Mutex<Option<JxlStreamInfo>>>,
    }

    impl FixedBackend {
        fn with(supported: bool, outcome: Result<JxlDecoded>) -> Self {
            Self {
                supported,
                outcome,
                supports_calls: Arc::new(AtomicUsize::new(0)),
                decode_calls: Arc::new(AtomicUsize::new(0)),
                seen: Arc::new(std::sync::Mutex::new(None)),
            }
        }

        /// A backend that accepts everything and returns a `Gray8` raster of `fill`.
        fn returning(fill: u8) -> Self {
            Self::with(
                true,
                JxlDecoded::new(
                    PixelFormat::Gray8,
                    Dimensions::new(2, 2).unwrap(),
                    JxlOwnedSamples::U8(vec![fill; 4]),
                ),
            )
        }

        fn counters(&self) -> (Arc<AtomicUsize>, Arc<AtomicUsize>) {
            (
                Arc::clone(&self.supports_calls),
                Arc::clone(&self.decode_calls),
            )
        }

        fn seen(&self) -> Arc<std::sync::Mutex<Option<JxlStreamInfo>>> {
            Arc::clone(&self.seen)
        }
    }

    impl JxlCodestreamDecoder for FixedBackend {
        fn supports(&mut self, info: &JxlStreamInfo) -> bool {
            self.supports_calls.fetch_add(1, Ordering::SeqCst);
            *self.seen.lock().expect("test lock") = Some(*info);
            self.supported
        }

        fn decode(&mut self, _info: &JxlStreamInfo, _codestream: &[u8]) -> Result<JxlDecoded> {
            self.decode_calls.fetch_add(1, Ordering::SeqCst);
            match &self.outcome {
                Ok(decoded) => Ok(decoded.clone()),
                Err(error) if error.kind() == gamut_core::ErrorKind::Unsupported => {
                    Err(Error::unsupported(
                        env!("CARGO_PKG_NAME"),
                        error
                            .static_message()
                            .unwrap_or("JXL: test backend refusal"),
                    ))
                }
                Err(error) if error.kind() == gamut_core::ErrorKind::InvalidInput => {
                    Err(Error::invalid_input(
                        env!("CARGO_PKG_NAME"),
                        error
                            .static_message()
                            .unwrap_or("JXL: test backend failure"),
                    ))
                }
                Err(_) => Err(Error::invalid_input(
                    env!("CARGO_PKG_NAME"),
                    "JXL: test backend failure",
                )),
            }
        }
    }

    /// A minimal bare codestream signature, enough for framing detection.
    const STREAM: [u8; 2] = [0xFF, 0x0A];

    #[test]
    fn first_supporting_backend_wins_and_later_ones_are_untouched() {
        let first = FixedBackend::returning(7);
        let second = FixedBackend::returning(9);
        let (second_supports, second_decodes) = second.counters();

        let mut dec = JxlDecoder::new();
        dec.push_backend(first).push_backend(second);

        let image: ImageBuf<Gray8> = dec.decode_image(&STREAM).expect("decode");
        assert_eq!(image.as_samples(), &[7, 7, 7, 7]);
        assert_eq!(image.dimensions(), Dimensions::new(2, 2).unwrap());
        assert_eq!(second_supports.load(Ordering::SeqCst), 0);
        assert_eq!(second_decodes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_declining_backend_is_skipped_in_favour_of_the_next() {
        let first = FixedBackend::with(
            false,
            JxlDecoded::new(
                PixelFormat::Gray8,
                Dimensions::new(2, 2).unwrap(),
                JxlOwnedSamples::U8(vec![1; 4]),
            ),
        );
        let (first_supports, first_decodes) = first.counters();
        let second = FixedBackend::returning(3);
        let (second_supports, second_decodes) = second.counters();

        let mut dec = JxlDecoder::new();
        dec.push_backend(first).push_backend(second);

        let image: ImageBuf<Gray8> = dec.decode_image(&STREAM).expect("decode");
        assert_eq!(image.as_samples(), &[3, 3, 3, 3]);
        assert_eq!(first_supports.load(Ordering::SeqCst), 1);
        assert_eq!(first_decodes.load(Ordering::SeqCst), 0);
        assert_eq!(second_supports.load(Ordering::SeqCst), 1);
        assert_eq!(second_decodes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_late_unsupported_falls_through_to_the_next_backend() {
        let first = FixedBackend::with(
            true,
            Err(Error::unsupported(
                env!("CARGO_PKG_NAME"),
                "changed its mind",
            )),
        );
        let (_, first_decodes) = first.counters();
        let second = FixedBackend::returning(5);
        let (_, second_decodes) = second.counters();

        let mut dec = JxlDecoder::new();
        dec.push_backend(first).push_backend(second);

        let image: ImageBuf<Gray8> = dec.decode_image(&STREAM).expect("decode");
        assert_eq!(image.as_samples(), &[5, 5, 5, 5]);
        assert_eq!(first_decodes.load(Ordering::SeqCst), 1);
        assert_eq!(second_decodes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn an_accepted_then_failed_backend_propagates_and_stops_the_chain() {
        let first = FixedBackend::with(
            true,
            Err(Error::invalid_input(
                env!("CARGO_PKG_NAME"),
                "JXL: test backend failure",
            )),
        );
        let second = FixedBackend::returning(6);
        let (second_supports, second_decodes) = second.counters();

        let mut dec = JxlDecoder::new();
        dec.push_backend(first).push_backend(second);

        let result: Result<ImageBuf<Gray8>> = dec.decode_image(&STREAM);
        let error = result.unwrap_err();
        assert_eq!(error.kind(), gamut_core::ErrorKind::InvalidInput);
        assert_eq!(error.static_message(), Some("JXL: test backend failure"));
        // Neither a later backend nor the built-in tail was reached.
        assert_eq!(second_supports.load(Ordering::SeqCst), 0);
        assert_eq!(second_decodes.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_backend_returning_the_wrong_layout_is_a_typed_error() {
        // The backend answers a Gray8 request with an Rgb8 raster; the host refuses to reshape it.
        let backend = FixedBackend::with(
            true,
            JxlDecoded::new(
                PixelFormat::Rgb8,
                Dimensions::new(2, 2).unwrap(),
                JxlOwnedSamples::U8(vec![1; 12]),
            ),
        );
        let mut dec = JxlDecoder::new();
        dec.push_backend(backend);
        let result: Result<ImageBuf<Gray8>> = dec.decode_image(&STREAM);
        let error = result.unwrap_err();
        assert_eq!(error.kind(), gamut_core::ErrorKind::InvalidInput);
        assert_eq!(
            error.static_message(),
            Some("JXL: backend returned a raster in the wrong pixel layout")
        );
    }

    #[test]
    fn the_backend_sees_the_requested_layout_framing_and_policy() {
        let backend = FixedBackend::returning(1);
        let seen = backend.seen();
        let mut dec = JxlDecoder::new().with_codestream_bit_depth(true);
        dec.push_backend(backend);

        let _: Result<ImageBuf<Gray8>> = dec.decode_image(&STREAM);
        let info = seen
            .lock()
            .expect("test lock")
            .expect("supports was called");
        assert_eq!(info.format(), PixelFormat::Gray8);
        assert_eq!(info.framing(), JxlFraming::Codestream);
        assert!(info.codestream_bit_depth());

        // A container-framed stream is reported as such; junk is Unknown.
        let container = [
            0x00, 0x00, 0x00, 0x0C, 0x4A, 0x58, 0x4C, 0x20, 0x0D, 0x0A, 0x87, 0x0A,
        ];
        let _: Result<ImageBuf<Gray8>> = dec.decode_image(&container);
        assert_eq!(
            seen.lock().expect("test lock").expect("info").framing(),
            JxlFraming::IsoBmff
        );
        let _: Result<ImageBuf<Gray8>> = dec.decode_image(&[0x01, 0x02]);
        assert_eq!(
            seen.lock().expect("test lock").expect("info").framing(),
            JxlFraming::Unknown
        );
    }

    #[test]
    fn decode_image_into_replaces_the_destination_from_a_backend() {
        let mut dec = JxlDecoder::new();
        dec.push_backend(FixedBackend::returning(8));
        let mut dst: ImageBuf<Gray8> =
            ImageBuf::new(vec![0u8; 9], Dimensions::new(3, 3).unwrap()).unwrap();
        dec.decode_image_into(&STREAM, &mut dst).expect("decode");
        assert_eq!(dst.dimensions(), Dimensions::new(2, 2).unwrap());
        assert_eq!(dst.as_samples(), &[8, 8, 8, 8]);
    }

    #[test]
    fn sixteen_bit_backend_output_reaches_the_caller() {
        let mut dec = JxlDecoder::new();
        dec.push_backend(FixedBackend::with(
            true,
            JxlDecoded::new(
                PixelFormat::Gray16,
                Dimensions::new(2, 2).unwrap(),
                JxlOwnedSamples::U16(vec![0xBEEF; 4]),
            ),
        ));
        let image: ImageBuf<Gray16> = dec.decode_image(&STREAM).expect("decode");
        assert_eq!(image.as_samples(), &[0xBEEF; 4]);
    }

    #[test]
    fn with_no_backend_the_builtin_tail_decides() {
        // The wasm-shaped story asserted on the dispatcher: an empty registry means the built-in
        // tail answers, and its absence is a typed refusal rather than a panic.
        let dec = JxlDecoder::new();
        let result: Result<ImageBuf<Gray8>> = dec.decode_image(&STREAM);
        // A two-byte signature is never a decodable image either way, so both builds error; only
        // the *kind* differs.
        let error = result.expect_err("a bare signature is not an image");
        if cfg!(feature = "decode") {
            assert_eq!(error.kind(), gamut_core::ErrorKind::InvalidInput);
        } else {
            assert_eq!(error.kind(), gamut_core::ErrorKind::Unsupported);
            assert_eq!(
                error.static_message(),
                Some(
                    "JXL: no decode backend (enable the `decode` feature or push a codestream backend)"
                )
            );
        }
    }

    #[test]
    fn all_backends_declining_falls_through_to_the_tail() {
        let first = FixedBackend::with(
            false,
            Err(Error::unsupported(env!("CARGO_PKG_NAME"), "never called")),
        );
        let (first_supports, _) = first.counters();
        let second = FixedBackend::with(
            true,
            Err(Error::unsupported(env!("CARGO_PKG_NAME"), "late decline")),
        );
        let (_, second_decodes) = second.counters();

        let mut dec = JxlDecoder::new();
        dec.push_backend(first).push_backend(second);

        let result: Result<ImageBuf<Gray8>> = dec.decode_image(&STREAM);
        assert_eq!(first_supports.load(Ordering::SeqCst), 1);
        assert_eq!(second_decodes.load(Ordering::SeqCst), 1);
        // Reaching the tail with a two-byte signature errors, but as the tail's error.
        let error = result.expect_err("a bare signature is not an image");
        if cfg!(feature = "decode") {
            assert_eq!(error.kind(), gamut_core::ErrorKind::InvalidInput);
        } else {
            assert_eq!(error.kind(), gamut_core::ErrorKind::Unsupported);
        }
    }

    #[test]
    fn clones_share_one_registry() {
        let mut dec = JxlDecoder::new();
        let clone = dec.clone();
        dec.push_backend(FixedBackend::returning(2));
        assert!(!clone.backends.is_empty());
        // Equality still compares configuration only.
        assert_eq!(dec, clone);
        assert!(format!("{dec:?}").contains("backends: 1"));
    }
}
