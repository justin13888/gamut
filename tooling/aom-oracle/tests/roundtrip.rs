//! Self-test for the libaom reference-encoder scaffold: a **lossless** all-intra encode must
//! decode back bit-exact through the reference decoder. This exercises both FFI directions
//! (`encode_still_intra` → `decode_av1`) so the encode path is not dead code, and pins the
//! reference-encoder oracle that the future gamut AV1 decoder will validate against.

/// A textured 8-bit 4:4:4 image: three independent Y/U/V ramps so a plane swap would be caught.
/// All arithmetic is `u8`-wrapping (widths here stay ≤ 255), so it never overflows.
fn planes(w: u32, h: u32) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let (mut y, mut u, mut v) = (Vec::new(), Vec::new(), Vec::new());
    for row in 0..h {
        for col in 0..w {
            let (c, r) = (col as u8, row as u8);
            y.push(c.wrapping_mul(7).wrapping_add(r.wrapping_mul(3)));
            u.push(c.wrapping_sub(r).wrapping_mul(5));
            v.push(r.wrapping_mul(11).wrapping_add(c.wrapping_mul(2)));
        }
    }
    (y, u, v)
}

#[test]
fn lossless_encode_decode_roundtrip() {
    let (w, h) = (64u32, 48u32);
    let (y, u, v) = planes(w, h);

    let stream = aom_oracle::encode_still_intra(w, h, &y, &u, &v, 0)
        .expect("libaom reference lossless encode");
    let decoded = aom_oracle::decode_av1(&stream).expect("libaom reference decode");

    assert_eq!(
        (decoded.width, decoded.height),
        (w, h),
        "dimensions round-trip"
    );
    assert_eq!(decoded.bit_depth, 8, "8-bit round-trip");

    let widen = |p: &[u8]| p.iter().map(|&s| u16::from(s)).collect::<Vec<u16>>();
    assert_eq!(
        decoded.planes[0],
        widen(&y),
        "Y plane is bit-exact under lossless"
    );
    assert_eq!(
        decoded.planes[1],
        widen(&u),
        "U plane is bit-exact under lossless"
    );
    assert_eq!(
        decoded.planes[2],
        widen(&v),
        "V plane is bit-exact under lossless"
    );
}

#[test]
fn lossy_encode_is_decodable() {
    // A lossy encode need not be bit-exact, but must produce a well-formed, decodable stream of
    // the right dimensions — the shape a decoder-validation harness depends on.
    let (w, h) = (32u32, 32u32);
    let (y, u, v) = planes(w, h);

    let stream = aom_oracle::encode_still_intra(w, h, &y, &u, &v, 160)
        .expect("libaom reference lossy encode");
    let decoded = aom_oracle::decode_av1(&stream).expect("libaom reference decode");

    assert_eq!((decoded.width, decoded.height), (w, h));
    assert_eq!(decoded.planes[0].len(), (w * h) as usize);
}
