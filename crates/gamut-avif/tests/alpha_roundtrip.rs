//! The alpha and grayscale input surface: encode `Rgba8`/`Gray8`, then check both what the
//! container says and what a real AVIF reader makes of it.
//!
//! Two independent questions, so two kinds of assertion. *Structure* — the auxiliary item is
//! hidden, essential-`auxC`-typed, `auxl`-linked, monochrome and `colr`-free — is read back with
//! this crate's own parser, because it is the file layout AVIF v1.2.0 §4 prescribes. *Meaning* —
//! that a reader recovers the caller's alpha — is asked of **libavif** (dav1d backend), which
//! merges the auxiliary into its RGBA presentation exactly as a browser would.
//!
//! libavif is linked in from the `third_party/libavif` + `third_party/dav1d` submodules via the
//! `libavif-oracle` dev-dependency, so the check is hermetic. Building it needs cmake/meson/ninja
//! and the checked-out submodules (`git submodule update --init --recursive` plus `mise run
//! fetch-av1-oracles`).

use gamut_avif::{AvifContainer, AvifEncoder, AvifItem, ChromaFormat};
use gamut_color::{ColourPrimaries, MatrixCoefficients, TransferCharacteristics};
use gamut_core::{Dimensions, EncodeImage, Gray8, ImageRef, Rgb8, Rgba8};

const W: u32 = 34;
const H: u32 = 18;

const DIMS: Dimensions = Dimensions {
    width: W,
    height: H,
};

/// The `aux_type` URN AVIF v1.2.0 §4 assigns an alpha auxiliary. Spelled out here rather than
/// imported so the test pins the wire string independently of the constant the encoder uses.
const ALPHA_URN: &str = "urn:mpeg:mpegB:cicp:systems:auxiliary:alpha";

/// A source whose alpha is **structured and non-trivial**: a diagonal ramp with fully transparent
/// and fully opaque runs, so an encoder that dropped, replicated, or transposed the plane is
/// caught. Colour varies independently of alpha, so the two cannot be confused for one another.
fn rgba_at(x: u32, y: u32) -> [u8; 4] {
    [
        ((x * 7 + y * 3) & 0xff) as u8,
        ((x * x + y) & 0xff) as u8,
        ((x ^ (y * 5)) & 0xff) as u8,
        ((x * 11 + y * 29) % 256) as u8,
    ]
}

fn source_rgba() -> Vec<u8> {
    let mut px = vec![0u8; (W * H * 4) as usize];
    for y in 0..H {
        for x in 0..W {
            let i = ((y * W + x) * 4) as usize;
            px[i..i + 4].copy_from_slice(&rgba_at(x, y));
        }
    }
    px
}

fn gray_at(x: u32, y: u32) -> u8 {
    ((x * 13 + y * 7) & 0xff) as u8
}

fn source_gray() -> Vec<u8> {
    let mut px = vec![0u8; (W * H) as usize];
    for y in 0..H {
        for x in 0..W {
            px[(y * W + x) as usize] = gray_at(x, y);
        }
    }
    px
}

fn encode_rgba(encoder: &AvifEncoder) -> Vec<u8> {
    let px = source_rgba();
    encoder
        .encode_to_vec(ImageRef::<Rgba8>::new(&px, DIMS).expect("buffer matches dimensions"))
        .expect("encode")
}

fn encode_gray(encoder: &AvifEncoder) -> Vec<u8> {
    let px = source_gray();
    encoder
        .encode_to_vec(ImageRef::<Gray8>::new(&px, DIMS).expect("buffer matches dimensions"))
        .expect("encode")
}

/// The items `item` points at with a `ref_type` item reference, if it owns one. Reads the raw
/// `iref` vector, so the *direction* of a reference is observable — the crate's own lenses answer
/// "is there one", which cannot distinguish the two ends.
fn targets<'a>(item: &'a gamut_isobmff::Item, ref_type: &[u8; 4]) -> Option<&'a [u32]> {
    item.references
        .iter()
        .find(|r| &r.reference_type == ref_type)
        .map(|r| &r.to_item_ids[..])
}

/// The `av1C` chroma format an item declares.
fn chroma(item: &AvifItem<'_>) -> ChromaFormat {
    item.av1_config()
        .expect("an av01 item has an av1C")
        .expect("the record parses")
        .chroma_format()
}

// ---- structure --------------------------------------------------------------------------------

#[test]
fn alpha_is_a_conformant_auxiliary_image_item() {
    let avif = encode_rgba(&AvifEncoder::new());
    let container = AvifContainer::parse(&avif).expect("our own reader parses the file");
    let img = container.image();

    // Exactly two items: the colour image and its alpha. The primary is still id 1, so attaching
    // alpha does not renumber anything a metadata item already depended on.
    assert_eq!(img.items().count(), 2);
    let colour = img.primary_item();
    assert_eq!(colour.id(), 1);
    assert_eq!(colour.bits_per_channel(), Some(&[8u8, 8, 8][..]));
    assert_eq!(chroma(&colour), ChromaFormat::Yuv444);

    // The alpha auxiliary is reached the way a reader reaches it: an `auxl` reference to the
    // colour item, typed by the alpha URN.
    let alpha = img
        .alpha_auxiliary_of(colour.id())
        .expect("the colour item has an alpha auxiliary");
    assert_eq!(alpha.id(), 2);
    assert_eq!(alpha.auxiliary_type(), Some(ALPHA_URN));
    assert_eq!(
        alpha.dimensions(),
        Some(DIMS),
        "aux ispe matches the master"
    );

    // AVIF v1.2.0 §4.1: an AV1 auxiliary image item shall be monochrome, and its `pixi` describes
    // the one channel it actually codes.
    assert_eq!(chroma(&alpha), ChromaFormat::Monochrome);
    assert_eq!(alpha.bits_per_channel(), Some(&[8u8][..]));
    // §4.1: `colr` should be omitted — the samples are opacity, not colour.
    assert!(alpha.colour().is_none(), "no colr on an alpha item");

    let raw = alpha.as_isobmff_item();
    // Hidden, because an alpha plane is not independently displayable.
    assert!(raw.hidden);
    // The `auxC` type is *essential*: a reader that does not understand it must refuse the item
    // rather than display an opacity map as a picture.
    let aux_property = raw
        .properties
        .iter()
        .find(|p| matches!(p.kind, gamut_isobmff::PropertyKind::AuxiliaryType { .. }))
        .expect("auxC present");
    assert!(aux_property.essential, "auxC must be essential");
    // `auxl` runs auxiliary → master, so the reference lives on the auxiliary.
    assert_eq!(targets(raw, b"auxl"), Some(&[1u32][..]));
    assert_eq!(
        targets(colour.as_isobmff_item(), b"auxl"),
        None,
        "the colour item does not own the auxl reference"
    );
}

#[test]
fn premultiplication_is_declared_only_when_asked() {
    // `prem` runs the other way to `auxl` — colour image → alpha auxiliary — so getting the
    // direction wrong is invisible to a presence check on the wrong item.
    for premultiplied in [false, true] {
        let avif = encode_rgba(&AvifEncoder::new().with_premultiplied_alpha(premultiplied));
        let img = AvifContainer::parse(&avif).expect("parses");
        let img = img.image();
        assert_eq!(img.is_premultiplied(1), premultiplied);
        assert_eq!(
            targets(img.primary_item().as_isobmff_item(), b"prem"),
            premultiplied.then_some(&[2u32][..]),
            "prem targets the alpha item, from the colour item"
        );
        // libavif reads the same reference, so the declaration is not private to our own model.
        let oracle = libavif_oracle::introspect(&avif).expect("libavif parses");
        assert_eq!(oracle.premultiplied_alpha, premultiplied);
    }
    // Without an alpha channel there is nothing to premultiply, and the knob reaches no box.
    let rgb = vec![0u8; (W * H * 3) as usize];
    let avif = AvifEncoder::new()
        .with_premultiplied_alpha(true)
        .encode_to_vec(ImageRef::<Rgb8>::new(&rgb, DIMS).unwrap())
        .expect("encode");
    assert!(
        !AvifContainer::parse(&avif)
            .unwrap()
            .image()
            .is_premultiplied(1)
    );
}

#[test]
fn grayscale_is_one_monochrome_item() {
    let avif = encode_gray(&AvifEncoder::new());
    let container = AvifContainer::parse(&avif).expect("parses");
    let img = container.image();
    assert_eq!(img.items().count(), 1, "no replicated chroma, no auxiliary");
    let primary = img.primary_item();
    assert_eq!(chroma(&primary), ChromaFormat::Monochrome);
    // One channel declared, because one is coded — three would claim colour the item does not hold.
    assert_eq!(primary.bits_per_channel(), Some(&[8u8][..]));
    assert!(img.alpha_auxiliary_of(primary.id()).is_none());
}

#[test]
fn a_monochrome_item_costs_the_advanced_profile_brand() {
    // AVIF v1.2.0 §8.3 constrains *every* AV1 image item in an `MA1A` file to the High Profile,
    // and a monochrome item is Main (profile 0). §8.1 blesses signalling only the general brands
    // when no profile fits, which is what an alpha or grayscale file does.
    let rgb = vec![0u8; (W * H * 3) as usize];
    let colour_only = AvifEncoder::new()
        .encode_to_vec(ImageRef::<Rgb8>::new(&rgb, DIMS).unwrap())
        .expect("encode");
    for (name, avif, advanced) in [
        ("rgb", colour_only, true),
        ("rgba", encode_rgba(&AvifEncoder::new()), false),
        ("gray", encode_gray(&AvifEncoder::new()), false),
    ] {
        let container = AvifContainer::parse(&avif).expect("parses");
        let brands = container.image().compatible_brands().to_vec();
        assert!(
            brands.starts_with(&[*b"avif", *b"mif1", *b"miaf"]),
            "{name}: general brands are always signalled"
        );
        assert_eq!(
            brands.contains(b"MA1A"),
            advanced,
            "{name}: MA1A claim, brands {brands:?}"
        );
        // The Baseline brand is never claimed in its place: it carries MIAF constraints this
        // encoder does not check.
        assert!(!brands.contains(b"MA1B"), "{name}");
    }
}

// ---- meaning, per libavif ---------------------------------------------------------------------

#[test]
fn lossless_rgba_round_trips_through_libavif() {
    let avif = encode_rgba(&AvifEncoder::new());
    let oracle = libavif_oracle::introspect(&avif).expect("libavif parses");
    assert!(oracle.alpha_present);
    assert!(!oracle.premultiplied_alpha);

    // libavif's own RGBA presentation: it decodes both items and merges the auxiliary. Lossless
    // identity coding means every channel — alpha included — must come back exactly.
    let (w, h, rgba) = libavif_oracle::decode_rgba(&avif).expect("libavif decodes");
    assert_eq!((w, h), (W, H));
    assert_eq!(rgba, source_rgba());

    // `decode_avif` leaves libavif in its **strict** mode, unlike `introspect`/`decode_rgba` — so
    // this also puts the file through the reader's own conformance checks on an alpha item, the
    // `ispe` requirement among them.
    let strict = libavif_oracle::decode_avif(&avif).expect("strict libavif decode");
    assert!(!strict.monochrome, "the colour item stays 4:4:4");
}

#[test]
fn lossless_grayscale_round_trips_through_libavif() {
    let avif = encode_gray(&AvifEncoder::new());
    let decoded = libavif_oracle::decode_avif(&avif).expect("libavif decodes");
    assert_eq!((decoded.width, decoded.height), (W, H));
    assert!(decoded.monochrome, "libavif reports one coded plane");
    let expected: Vec<u16> = source_gray().into_iter().map(u16::from).collect();
    assert_eq!(decoded.planes[0], expected);
    assert!(decoded.planes[1].is_empty() && decoded.planes[2].is_empty());
}

#[test]
fn a_monochrome_item_still_carries_the_primaries_and_transfer_tags() {
    // `with_primaries`/`with_transfer` only *tag* samples, so they apply to a monochrome item just
    // as they do to a colour one — and they reach both the AV1 sequence header and `colr`, which
    // §2.3.4 requires to agree. What does *not* apply is the matrix: there is no chroma to
    // describe, and AV1 §6.4.2 forbids `MC_IDENTITY` on a single-plane stream, so the encoder
    // signals `Unspecified` however `with_matrix` was set.
    let avif = encode_gray(
        &AvifEncoder::new()
            .with_primaries(ColourPrimaries::Bt2020)
            .with_transfer(TransferCharacteristics::Pq)
            .with_matrix(MatrixCoefficients::Bt709),
    );
    let oracle = libavif_oracle::introspect(&avif).expect("libavif parses");
    assert_eq!(oracle.color_primaries, ColourPrimaries::Bt2020.code_point());
    assert_eq!(
        oracle.transfer_characteristics,
        TransferCharacteristics::Pq.code_point()
    );
    assert_eq!(
        oracle.matrix_coefficients,
        MatrixCoefficients::Unspecified.code_point()
    );
    // AVIF v1.2.0 §4.1 makes full range a *shall* for an auxiliary item, and a monochrome AV1
    // stream codes the bit explicitly rather than inferring it.
    assert!(oracle.full_range);
}

#[test]
fn lossy_alpha_is_the_av1_reconstruction_of_the_alpha_plane() {
    // Lossy alpha is coded, not dropped or forced opaque — and coded at the *colour item's*
    // `base_q_idx`, as a monochrome still. The exact assertion is available: quantized output is
    // not the source, but it must be the AV1 encoder's own reconstruction of the same plane, which
    // is what a conformant decoder is obliged to reproduce. A tolerance would pass just as well
    // for an alpha plane quantized at the wrong index, or coded from the wrong channel.
    const QUALITY: u8 = 80;
    let avif = encode_rgba(&AvifEncoder::lossy(QUALITY));
    assert!(libavif_oracle::introspect(&avif).unwrap().alpha_present);

    let px = source_rgba();
    let alpha_planes =
        gamut_color::Planar8::from_rgba8_alpha_view(ImageRef::<Rgba8>::new(&px, DIMS).unwrap());
    let (_, recon) = gamut_av1::encode_still_intra_with(
        &alpha_planes,
        quality_to_quant(QUALITY),
        gamut_av1::Av1Colour::monochrome(),
    )
    .expect("the alpha plane encodes");

    let (_, _, rgba) = libavif_oracle::decode_rgba(&avif).expect("libavif decodes");
    let merged: Vec<u16> = rgba
        .as_chunks::<4>()
        .0
        .iter()
        .map(|px| u16::from(px[3]))
        .collect();
    assert_eq!(merged, recon.planes[0]);
    // And it is genuinely varying alpha, not a constant plane that would match trivially.
    assert!(merged.iter().any(|&a| a < 200) && merged.iter().any(|&a| a > 200));
}

/// Mirrors `gamut-avif`'s documented quality→`base_q_idx` mapping, as `decode_roundtrip.rs` does:
/// the test needs the exact index the encoder selected in order to reproduce its reconstruction.
fn quality_to_quant(quality: u8) -> u8 {
    debug_assert!(quality <= 100, "quality must be 0..=100, got {quality}");
    (((100 - u32::from(quality)) * 255 / 100) as u8).max(1)
}

#[test]
fn odd_and_tiny_sizes_round_trip_with_alpha() {
    // Sizes that exercise edge padding and a single-superblock frame, where a plane-geometry
    // mistake in the monochrome auxiliary would show up first.
    for (w, h) in [(1, 1), (3, 7), (17, 13), (64, 64)] {
        let dims = Dimensions {
            width: w,
            height: h,
        };
        let mut px = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                px[i..i + 4].copy_from_slice(&rgba_at(x, y));
            }
        }
        let avif = AvifEncoder::new()
            .encode_to_vec(ImageRef::<Rgba8>::new(&px, dims).unwrap())
            .unwrap_or_else(|e| panic!("encode {w}x{h}: {e}"));
        let (dw, dh, rgba) = libavif_oracle::decode_rgba(&avif)
            .unwrap_or_else(|e| panic!("libavif decode {w}x{h}: {e}"));
        assert_eq!((dw, dh), (w, h));
        assert_eq!(rgba, px, "{w}x{h}");
    }
}
