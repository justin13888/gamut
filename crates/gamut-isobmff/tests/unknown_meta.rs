//! Meta-level accounting for [`walk_meta_children`]: boxes inside `meta`/`iprp` that the semantic
//! parse does not consume are surfaced verbatim, and the consumed ones are never double-reported.
//!
//! Pinned against `walk_meta_children` directly rather than through a container's `parse`, so the
//! reach is the shadow walk alone. `gamut-avif` and `gamut-heic` each carried a copy of this file
//! whose normalised diff from the other was three comment rewraps (#436).

mod common;

use common::{bx, cat, full, hdlr, iinf_v0, infe_v2, meta, pitm_v0};
use gamut_isobmff::{UnknownBoxLocation, walk_meta_children};

/// A hand-authored `meta` carrying an unconsumed `uuid` child, whose `iprp` carries an unconsumed
/// `free` child alongside the real `ipco`/`ipma`. Both unknowns must surface with their exact
/// bodies while the known children stay unreported.
#[test]
fn unknown_meta_and_iprp_children_surface_verbatim() {
    let ispe = full(
        b"ispe",
        0,
        0,
        &cat(&[64u32.to_be_bytes(), 48u32.to_be_bytes()]),
    );
    let ipco = bx(b"ipco", &ispe);
    let ipma = full(
        b"ipma",
        0,
        0,
        &cat(&[
            &1u32.to_be_bytes()[..], // entry_count
            &1u16.to_be_bytes(),     // item_ID
            &[1u8],                  // association_count
            &[0x01],                 // essential 0 | index 1
        ]),
    );
    // The stray iprp child (not ipco/ipma).
    let iprp_stray_body = b"stray-in-iprp".to_vec();
    let iprp = bx(b"iprp", &cat(&[ipco, ipma, bx(b"free", &iprp_stray_body)]));

    // The stray meta child (not a consumed box type): a `uuid` box (16-byte UUID + data).
    let uuid_body = cat(&[&[0u8; 16][..], b"vendor-uuid-payload"]);
    let uuid = bx(b"uuid", &uuid_body);

    let m = meta(&[
        hdlr(),
        pitm_v0(1),
        iinf_v0(&[infe_v2(1, b"av01")]),
        iprp,
        uuid,
    ]);
    // `walk_meta_children` takes the meta *body*: strip the 8-byte box header.
    let unknown = walk_meta_children(&m[8..]).unwrap();

    assert_eq!(unknown.len(), 2, "exactly the two unconsumed boxes");

    let uuid_box = unknown
        .iter()
        .find(|b| &b.ty == b"uuid")
        .expect("uuid captured");
    assert_eq!(uuid_box.location, UnknownBoxLocation::Meta);
    assert_eq!(uuid_box.body, uuid_body.as_slice());

    let iprp_box = unknown
        .iter()
        .find(|b| &b.ty == b"free")
        .expect("stray iprp child captured");
    assert_eq!(iprp_box.location, UnknownBoxLocation::Iprp);
    assert_eq!(iprp_box.body, iprp_stray_body.as_slice());

    // Consumed children (hdlr/pitm/iinf/iprp at meta level; ipco/ipma inside iprp) are NOT
    // reported.
    for consumed in [b"hdlr", b"pitm", b"iinf", b"iprp", b"ipco", b"ipma"] {
        assert!(
            !unknown.iter().any(|b| &b.ty == consumed),
            "consumed box {:?} must not be reported",
            core::str::from_utf8(consumed).unwrap()
        );
    }
}
