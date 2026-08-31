# C2PA (Coalition for Content Provenance and Authenticity) — content-credentials reference

Reference specifications for gamut's C2PA work, epic
[#239](https://github.com/visualcommons/gamut/issues/239).

**This directory is the one exception to "one reference directory, one owning crate."** C2PA is a
cross-format payload, not a format: the same JUMBF manifest store is carried by an ISOBMFF `uuid`
box, a PNG chunk, a TIFF tag, a RIFF chunk and a run of JPEG APP11 segments. Eight crates consume
clauses from here. The [clause map](#clause-map--which-crate-consumes-what) below names each one
against the clause it implements.

## The scope boundary this directory does *not* cover

gamut **locates, bounds, carries and reserves** the manifest store. **Validation belongs to
[`c2pa-rs`](https://github.com/contentauth/c2pa-rs).** No COSE, X.509, RFC 3161 or trust-list code
exists in any shipped gamut crate, gamut never reports a validity verdict, and no gamut API implies
one. The store is opaque bytes plus byte ranges; the JUMBF interior — claims, CBOR, assertion
schemas, ingredient ancestry — is out of scope for #239.

Clauses §13 (signature layer) and §15.12 (validation-side rules) are therefore staked here for the
**boundary statement only**: they say what gamut must *not* claim to do, and they define the
exclusion contract a host must honour when it reserves space for an external signer.

## Vendored — the C2PA specification is CC BY 4.0

Unlike the ISO base standards below, the C2PA documents carry a Creative Commons licence and are
redistributable, so they are vendored verbatim. Retrieved 2026-08-31; upstream `last-modified`
2026-04-23.

| file | bytes | what it is |
| --- | --- | --- |
| [`C2PA_Specification_2.4.html`][spec-html] | 1,065,169 | canonical HTML; carries the clause anchors every gamut C2PA issue cites |
| [`C2PA_Specification_2.4.pdf`][spec-pdf] | 8,139,117 | the same text, paginated for stable citation (307 pages, "2.4, 2026-04-01") |
| [`C2PA_Schemas_2.4.zip`][spec-schemas] | 57,677 | the normative CDDL and JSON Schemas — 47 files: `cddl/*.cddl`, `c2pa_urn.abnf`, `valid_metadata_fields.yml`, the soft-binding OpenAPI and crJSON schemas |

[spec-html]: https://spec.c2pa.org/specifications/specifications/2.4/specs/C2PA_Specification.html
[spec-pdf]: https://spec.c2pa.org/specifications/specifications/2.4/specs/_attachments/C2PA_Specification.pdf
[spec-schemas]: https://spec.c2pa.org/specifications/specifications/2.4/specs/_attachments/C2PA_Schemas.zip

The 2.4 index page's download menu only lists documents up to 2.3, but both `_attachments/` URLs
above resolve. Nothing in #239 consumes the CDDL yet — it is vendored because the JUMBF-interior
work deferred out of #239 is the only thing that ever will, and by then the 2.4 attachment may be
gone.

> **Extracting the schemas zip:** seven of its 47 entries carry `../../../` path prefixes
> (`softbinding/partials/…`, `crJSON/partials/…`) reflecting the upstream site layout. Extract with
> `unzip -j`, or into a scratch directory — a plain `unzip` here writes outside this directory.

### Attribution

> Copyright © 2026 Coalition for Content Provenance and Authenticity (C2PA). Except where noted,
> the content is licensed under a Creative Commons Attribution 4.0 International (CC BY 4.0)
> license.

> THESE MATERIALS ARE PROVIDED "AS IS." The parties expressly disclaim any warranties (express,
> implied, or otherwise), including implied warranties of merchantability, non-infringement,
> fitness for a particular purpose, or title, related to the materials. The entire risk as to
> implementing or otherwise using the materials is assumed by the implementer and user. IN NO EVENT
> WILL THE PARTIES BE LIABLE TO ANY OTHER PARTY FOR LOST PROFITS OR ANY FORM OF INDIRECT, SPECIAL,
> INCIDENTAL, OR CONSEQUENTIAL DAMAGES OF ANY CHARACTER FROM ANY CAUSES OF ACTION OF ANY KIND WITH
> RESPECT TO THIS DELIVERABLE OR ITS GOVERNING AGREEMENT, WHETHER BASED ON BREACH OF CONTRACT, TORT
> (INCLUDING NEGLIGENCE), OR OTHERWISE, AND WHETHER OR NOT THE OTHER MEMBER HAS BEEN ADVISED OF THE
> POSSIBILITY OF SUCH DAMAGE.

## Not vendored — paywalled

- **ISO/IEC 19566-5:2023** — *JPEG universal metadata box format (JUMBF)*, edition 2, 2023-06. The
  container the manifest store **is**. CHF 135, <https://www.iso.org/standard/84635.html>. gamut
  needs **Annex D.2** (the JPEG APP11 field layout: `CI`/`En`/`Z`/`LBox`/`TBox`) and clause 4's
  general box framing.
- **ISO/IEC 18477-3** — *JPEG XT box file format*. §A.3.1 defines the C2PA APP11 segment by
  reference to this document; it is the source of the marker-segment framing.
- **ISO/IEC 18181-2:2024** — *JPEG XL container* (`jumb` box, clause 9.3), cited by §A.3.9.

## Clause map — which crate consumes what

| crate | clause | what it takes from here |
| --- | --- | --- |
| `gamut-metadata` | §9.1, §11.1.1, §15.12.1.1, §9.2.6 | one hard binding per standard manifest; the store is a JUMBF superbox; the exclusion contract that makes a copied-forward manifest invalid |
| `gamut-heic` | §A.5.1–§A.5.3, §A.5.6, §15.12.3 | top-level `uuid` box, user type `D8FEC3D6-…-C481`; `box_purpose` and the 8-byte merkle offset; BMFF excludes by box path, not byte offset |
| `gamut-isobmff` | §A.5.3 | placement: after `ftyp`, before the first `mdat` and before any `moov` |
| `gamut-avif` | §A.5 | same BMFF carriage; AVIF is named explicitly in §A.1 |
| `gamut-png` | §A.3.2 | `caBX` chunk — ancillary, private, not-safe-to-copy; should precede `IDAT` |
| `gamut-dng` | §A.3.6, §18.5.5 | tag 52545 / `0xCD41`, type 7, in the **last** IFD of the main chain; the entry's `count` field is a *second, disjoint* exclusion range |
| `gamut-tiff` | §A.3.6 | as DNG. §18.7.3.3 removed general box hash for TIFF, so `c2pa.hash.data` is the only binding |
| `gamut-riff` / `gamut-webp` | §A.3.7 | `C2PA` chunk, last sub-chunk of the first RIFF header chunk |
| `gamut-jpeg` | §A.3.1 | **descoped — see below** |

## Recorded decisions

### The JPEG APP11 slice is descoped until ISO/IEC 19566-5 is acquired

§A.3.1 defines the C2PA JPEG carriage entirely by reference to two paywalled documents:

> The C2PA Manifest Store shall be embedded as the data contained in an APP11 marker segment as
> defined in JPEG XT, ISO/IEC 18477-3. […] they shall be constructed as per the JPEG 1 standard and
> ISO 19566-5:2023, D.2.

So the CC BY text vendored here contains **no** normative field layout for the APP11 segments —
Annex D.2 is the only source, and it is behind CHF 135. Writing the segmentation against
`c2pa-rs`'s implementation instead would violate `AGENTS.md`'s *"specification as source of
truth"*. **The JPEG slice of #239 is therefore out of scope until 19566-5 is purchased.**

This blocks nothing else. Every other container — HEIC, ISOBMFF/AVIF, PNG, DNG, TIFF and
RIFF/WebP — has its complete carriage defined in the vendored CC BY text above.

### `En = 0x0211` is a `c2pa-rs` convention, not normative

`c2pa-rs` writes the JUMBF box instance number `En = 0x0211`, with the source comment *"can be any
unique ID, so we pick one that shouldn't conflict"*. It appears nowhere in the C2PA specification.
Should the JPEG slice ever be un-descoped, that value must be traced to 19566-5 Annex D.2 or chosen
by gamut — never transcribed from the reference implementation as though it were spec.

### The JUMBF box framing has only an incidental trace here

The general JUMBF box header — a 4-byte big-endian `LBox` length followed by a 4-byte `TBox` type —
is normatively defined in ISO/IEC 19566-5, not in this specification. The CC BY text describes it
only *incidentally*, in §8.4.2.3 while specifying the `c2sh` salt-hash box:

> a box length (LBox, as a 4-byte big-endian unsigned integer); a box type (TBox, 4-byte big-endian
> unsigned integer, with a value of `c2sh` (for C2PA salt hash))

That incidental description is the whole trace for reading a manifest store's **outer** length,
which `gamut-heic` relies on to trim the *"zero or more unused padding bytes"* §A.5.3 permits after
the store. Reading that one length field is framing, not JUMBF interior, and is the only JUMBF
value any gamut crate reads under #239. Recorded here so the dependency is visible: if the outer
framing ever needs more than a length, 19566-5 must be acquired first.

## Already vendored elsewhere — no action

The host-container specifications the clauses above layer onto are already in this tree:
[`references/jpeg`](../jpeg) (marker syntax, ITU-T T.81 §B.2.4.6) · [`references/png`](../png)
(chunk layout, 4.7.2) · [`references/isobmff`](../isobmff) and [`references/heif`](../heif)
(14496-12 box syntax) · [`references/tiff`](../tiff) (IFD entry layout) ·
[`references/webp`](../webp) (RIFF chunk order).

## Oracle

`c2pa-rs` (Apache-2.0 / MIT), **dev-only**, in a workspace-`exclude`d `tooling/` crate — the
[`tooling/gamut-dng-real-conformance`](../../tooling/gamut-dng-real-conformance) shape. Being a Rust
crate it needs no `third_party/` submodule. Build it `--no-default-features --features
rust_native_crypto`: the default `openssl` feature is pulled *vendored* and would compile OpenSSL
from C source.

Two properties of `c2pa-rs` 0.90.16 shaped how gamut splits the work:

- `Builder`'s `*_embeddable` path plus `composed_manifest(bytes, "application/c2pa")` yields raw
  manifest-store bytes for a host to embed itself, and `CallbackSigner` carries a `reserve_size` —
  it already models signing as reserve-then-fill, which is the seam gamut exposes.
- There is **no cheap parse-only mode**: `ValidationState::Invalid` is also what you get when
  verification is disabled, so `c2pa-rs` cannot serve as a "parse but don't judge" front end. That
  is why gamut owns the locate-and-bound step rather than shelling out for it.
- Do **not** build on `c2pa::jumbf_io` — its public functions name crate-private traits
  (`CAIRead`/`CAIReadWrite`), a private-in-public leak.
