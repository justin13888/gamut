//! 16-bit input → 10/12-bit AVIF: what the container claims, and what a real reader gets back.
//!
//! `Rgb16`/`Rgba16` carry samples on `gamut-core`'s full 16-bit scale while AV1 codes 8, 10 or 12,
//! so the slice's whole contract is the narrowing and the depth signalling that must agree with it
//! — the AV1 sequence header, `av1C`, and `pixi` all state the depth, and a reader that disagrees
//! with any of them decodes the wrong picture. libavif (dav1d backend) answers what a reader gets;
//! this crate's own parser answers what the boxes say; dav1d directly answers for the alpha
//! auxiliary, which libavif's RGBA presentation would have already narrowed back to 8 bits.
//!
//! Building this needs the `third_party/libavif` + `third_party/dav1d` submodules and a C
//! toolchain, like the crate's other oracle tests.

use gamut_avif::{
    Av1Config, Av1StillDecoder, AvifContainer, AvifEncoder, ChromaFormat, DecodedFrame,
};
use gamut_color::{BitDepth, ChromaSubsampling, MatrixCoefficients};
use gamut_core::{Dimensions, EncodeImage, Error, ImageRef, Result, Rgb16, Rgba16};

const W: u32 = 34;
const H: u32 = 18;

const DIMS: Dimensions = Dimensions {
    width: W,
    height: H,
};

/// [`Av1StillDecoder`] over the real dav1d decoder, as `conformance.rs` bridges it: the typed
/// `av1C` plus the raw item payload are assembled into one self-contained temporal unit and handed
/// to a Section-5 decoder — the same door a platform decoder enters by.
struct Dav1dDecoder;

impl Av1StillDecoder for Dav1dDecoder {
    fn decode_still(&mut self, config: &Av1Config, payload: &[u8]) -> Result<DecodedFrame> {
        let mut stream = Vec::new();
        config.full_stream(payload, &mut stream)?;
        let pic = dav1d_oracle::decode_obu(&stream)
            .map_err(|_| Error::InvalidInput("high-bitdepth: dav1d rejected the stream"))?;
        let [y, u, v] = pic.planes;
        DecodedFrame::new(
            pic.width,
            pic.height,
            pic.bit_depth,
            config.chroma_format(),
            y,
            u,
            v,
        )
    }
}

/// A source spanning the whole 16-bit range, with the **low** bits carrying structure of their own
/// so a narrowing that shifted by the wrong amount — or rounded instead of truncating — cannot
/// coincide with the right answer.
fn rgba16_at(x: u32, y: u32) -> [u16; 4] {
    [
        ((x * 1973 + y * 613) % 65536) as u16,
        ((x * 4099 + y * 271) % 65536) as u16,
        ((x ^ (y * 7)) * 997 % 65536) as u16,
        ((x * 331 + y * 2657) % 65536) as u16,
    ]
}

fn source_rgb16() -> Vec<u16> {
    let mut px = vec![0u16; (W * H * 3) as usize];
    for y in 0..H {
        for x in 0..W {
            let i = ((y * W + x) * 3) as usize;
            px[i..i + 3].copy_from_slice(&rgba16_at(x, y)[..3]);
        }
    }
    px
}

fn source_rgba16() -> Vec<u16> {
    let mut px = vec![0u16; (W * H * 4) as usize];
    for y in 0..H {
        for x in 0..W {
            let i = ((y * W + x) * 4) as usize;
            px[i..i + 4].copy_from_slice(&rgba16_at(x, y));
        }
    }
    px
}

/// The plane a conformant decoder must produce for channel `c` of the source at `bits`: the top
/// `bits` of each sample, in AV1's GBR order (Y=G, U=B, V=R).
fn expected_plane(plane: usize, bits: BitDepth) -> Vec<u16> {
    let channel = [1usize, 2, 0][plane];
    let shift = 16 - u32::from(bits.bits());
    (0..H)
        .flat_map(|y| (0..W).map(move |x| rgba16_at(x, y)[channel] >> shift))
        .collect()
}

fn encode_rgb16(encoder: &AvifEncoder) -> Vec<u8> {
    let px = source_rgb16();
    encoder
        .encode_to_vec(ImageRef::<Rgb16>::new(&px, DIMS).expect("buffer matches dimensions"))
        .expect("encode")
}

#[test]
fn lossless_rgb16_round_trips_at_the_coded_depth() {
    for bits in [BitDepth::Ten, BitDepth::Twelve] {
        let avif = encode_rgb16(&AvifEncoder::new().with_bit_depth(bits));

        // libavif's own decode, in its default *strict* mode.
        let decoded = libavif_oracle::decode_avif(&avif)
            .unwrap_or_else(|e| panic!("libavif decode failed at {bits:?}: {e}"));
        assert_eq!((decoded.width, decoded.height), (W, H));
        assert_eq!(decoded.bit_depth, bits.bits());
        for plane in 0..3 {
            assert_eq!(
                decoded.planes[plane],
                expected_plane(plane, bits),
                "{bits:?}: plane {plane}"
            );
        }
    }
}

#[test]
fn the_depth_is_stated_the_same_way_everywhere() {
    // `av1C`, `pixi`, the AV1 sequence header and libavif's reading of the file must all agree:
    // a reader that trusts any one of them alone decodes the same picture.
    for (bits, profile) in [(BitDepth::Ten, 1u8), (BitDepth::Twelve, 2)] {
        let avif = encode_rgb16(&AvifEncoder::new().with_bit_depth(bits));
        let container = AvifContainer::parse(&avif).expect("our own reader parses it");
        let img = container.image();
        let primary = img.primary_item();

        let config = primary
            .av1_config()
            .expect("an av01 item has an av1C")
            .expect("the record parses");
        assert_eq!(config.bit_depth(), bits.bits(), "{bits:?}: av1C");
        assert_eq!(config.chroma_format(), ChromaFormat::Yuv444);
        // §6.4.1: 12-bit of any plane count needs the Professional profile; 10-bit 4:4:4 stays High.
        assert_eq!(config.seq_profile, profile, "{bits:?}: seq_profile");

        // `pixi` describes the item's actual channels, so it follows the depth, not a constant 8.
        assert_eq!(
            primary.bits_per_channel(),
            Some(&[bits.bits(), bits.bits(), bits.bits()][..]),
            "{bits:?}: pixi"
        );

        // And the codestream itself: decoding through the `av1C` the container carries reproduces
        // the same depth, which is what makes the record a description rather than a claim.
        let frame = img
            .decode_item_planar(primary.id(), &mut Dav1dDecoder)
            .expect("dav1d decodes the primary");
        assert_eq!(frame.bit_depth(), bits.bits(), "{bits:?}: codestream");
        assert_eq!(frame.y(), expected_plane(0, bits), "{bits:?}: luma");

        assert_eq!(
            libavif_oracle::introspect(&avif)
                .expect("libavif parses")
                .depth,
            bits.bits(),
            "{bits:?}: libavif"
        );

        // AVIF §8.3 admits only High Profile items into an `MA1A` file, so the depth decides the
        // brand as a side effect of deciding the profile: 10-bit keeps it, 12-bit does not.
        let brands = img.compatible_brands();
        assert!(
            brands.starts_with(&[*b"avif", *b"mif1", *b"miaf"]),
            "{bits:?}"
        );
        assert_eq!(
            brands.contains(b"MA1A"),
            profile == 1,
            "{bits:?}: MA1A claim, brands {brands:?}"
        );
    }
}

#[test]
fn rgba16_carries_its_alpha_at_the_master_depth() {
    // AVIF v1.2.0 §4.1: an auxiliary image item shall be encoded at the master item's bit depth.
    // libavif's RGBA presentation narrows everything back to 8 bits, so the alpha plane is checked
    // through dav1d at its own depth instead.
    for bits in [BitDepth::Ten, BitDepth::Twelve] {
        let px = source_rgba16();
        let avif = AvifEncoder::new()
            .with_bit_depth(bits)
            .encode_to_vec(ImageRef::<Rgba16>::new(&px, DIMS).unwrap())
            .expect("encode");

        assert!(
            libavif_oracle::introspect(&avif)
                .expect("libavif parses")
                .alpha_present
        );
        let container = AvifContainer::parse(&avif).expect("parses");
        let img = container.image();
        let alpha = img
            .alpha_auxiliary_of(img.primary_item().id())
            .expect("the colour item has an alpha auxiliary");
        let config = alpha.av1_config().unwrap().unwrap();
        assert_eq!(config.bit_depth(), bits.bits(), "{bits:?}: aux av1C");
        assert_eq!(config.chroma_format(), ChromaFormat::Monochrome);
        assert_eq!(alpha.bits_per_channel(), Some(&[bits.bits()][..]));
        assert!(alpha.colour().is_none(), "no colr on an alpha item");

        let frame = img
            .decode_item_planar(alpha.id(), &mut Dav1dDecoder)
            .expect("dav1d decodes the auxiliary");
        let shift = 16 - u32::from(bits.bits());
        let want: Vec<u16> = (0..H)
            .flat_map(|y| (0..W).map(move |x| rgba16_at(x, y)[3] >> shift))
            .collect();
        assert_eq!(frame.y(), want, "{bits:?}: alpha plane");
        // And the colour item is untouched by the alpha's presence.
        assert_eq!(
            img.primary_item().bits_per_channel(),
            Some(&[bits.bits(), bits.bits(), bits.bits()][..])
        );
    }
}

#[test]
fn only_ten_and_twelve_bit_coding_is_offered() {
    // 8 would silently discard a 16-bit input's whole point, and 16 is not an AV1 depth (§6.4.1),
    // so both are refused rather than quietly reinterpreted.
    let px = source_rgb16();
    for bits in [BitDepth::Eight, BitDepth::Sixteen] {
        let err = AvifEncoder::new()
            .with_bit_depth(bits)
            .encode_to_vec(ImageRef::<Rgb16>::new(&px, DIMS).unwrap())
            .expect_err("rejected");
        assert_eq!(
            err.static_message(),
            Some(
                "AVIF: a 16-bit input codes at 10 or 12 bits (AV1 §6.4.1); \
                 select one with AvifEncoder::with_bit_depth"
            ),
            "{bits:?}"
        );
    }
    // The default needs no knob at all.
    assert_eq!(AvifEncoder::new().config().bit_depth, BitDepth::Twelve);
    let default_avif = AvifEncoder::new()
        .encode_to_vec(ImageRef::<Rgb16>::new(&px, DIMS).unwrap())
        .expect("encode");
    assert_eq!(libavif_oracle::introspect(&default_avif).unwrap().depth, 12);
}

#[test]
fn lossy_rgb16_decodes_at_the_coded_depth() {
    // Lossy output is not the source, but it must still be a *high-bit-depth* picture: a path that
    // quietly fell back to 8 bits would decode fine and look almost right.
    for bits in [BitDepth::Ten, BitDepth::Twelve] {
        let px = source_rgb16();
        let avif = AvifEncoder::lossy(70)
            .with_bit_depth(bits)
            .encode_to_vec(ImageRef::<Rgb16>::new(&px, DIMS).unwrap())
            .expect("encode");
        let decoded = libavif_oracle::decode_avif(&avif)
            .unwrap_or_else(|e| panic!("libavif decode failed at {bits:?}: {e}"));
        assert_eq!(decoded.bit_depth, bits.bits());
        // Samples above the 8-bit range must actually occur, or the depth would be decorative.
        assert!(
            decoded.planes[0].iter().any(|&v| v > 255),
            "{bits:?}: decoded luma never exceeds 8 bits"
        );
    }
}

/// `with_chroma` now reaches the 16-bit path, so a high-bit-depth item can be 4:2:2 or 4:2:0 —
/// and libavif, decoding it in its strict mode, must report both the depth *and* the layout the
/// container claims.
///
/// The two are coupled in the stream: §6.4.1 puts any 12-bit picture on profile 2, and §5.5.2 has
/// profile 2 *code* its subsampling pair only at 12 bits — everywhere else the layout is inferred
/// from the profile. So a 12-bit 4:2:0 file is the one configuration in which both subsampling bits
/// are written, and it is the one this pins.
#[test]
fn sixteen_bit_input_honours_the_requested_chroma() {
    for bits in [BitDepth::Ten, BitDepth::Twelve] {
        for (chroma, want) in [
            (ChromaSubsampling::Cs444, ChromaFormat::Yuv444),
            (ChromaSubsampling::Cs422, ChromaFormat::Yuv422),
            (ChromaSubsampling::Cs420, ChromaFormat::Yuv420),
        ] {
            let px = source_rgb16();
            let avif = AvifEncoder::lossy(70)
                .with_bit_depth(bits)
                .with_matrix(MatrixCoefficients::Bt709)
                .with_chroma(chroma)
                .encode_to_vec(ImageRef::<Rgb16>::new(&px, DIMS).unwrap())
                .unwrap_or_else(|e| panic!("encode at {bits:?}/{chroma:?} failed: {e}"));

            // What the container says.
            let container = AvifContainer::parse(&avif).expect("parse");
            let cfg = container
                .image()
                .primary_item()
                .av1_config()
                .expect("an av01 item has an av1C")
                .expect("the record parses");
            assert_eq!(
                cfg.chroma_format(),
                want,
                "{bits:?}/{chroma:?}: av1C layout"
            );
            assert_eq!(
                cfg.bit_depth(),
                bits.bits(),
                "{bits:?}/{chroma:?}: av1C depth"
            );

            // What a real reader gets. libavif reports its own notion of the geometry, so a
            // container that claimed a layout the payload did not carry would disagree here.
            let decoded = libavif_oracle::decode_avif(&avif)
                .unwrap_or_else(|e| panic!("libavif rejected {bits:?}/{chroma:?}: {e}"));
            assert_eq!(decoded.bit_depth, bits.bits());
            let (cw, ch) = chroma.chroma_dimensions(W, H);
            assert_eq!(
                decoded.planes[1].len(),
                (cw * ch) as usize,
                "{bits:?}/{chroma:?}: libavif's chroma plane size"
            );
        }
    }
}

/// The identity matrix is 4:4:4 whatever `with_chroma` says — §6.4.2 permits `MC_IDENTITY` only
/// there — so the knob is dropped rather than half-honoured, exactly as on the 8-bit path.
#[test]
fn the_identity_matrix_stays_four_four_four_at_high_bit_depth() {
    let px = source_rgb16();
    let avif = AvifEncoder::lossy(70)
        .with_bit_depth(BitDepth::Twelve)
        .with_matrix(MatrixCoefficients::Identity)
        .with_chroma(ChromaSubsampling::Cs420)
        .encode_to_vec(ImageRef::<Rgb16>::new(&px, DIMS).unwrap())
        .expect("encode");
    let container = AvifContainer::parse(&avif).expect("parse");
    let cfg = container
        .image()
        .primary_item()
        .av1_config()
        .expect("an av01 item has an av1C")
        .expect("the record parses");
    assert_eq!(cfg.chroma_format(), ChromaFormat::Yuv444);
}
