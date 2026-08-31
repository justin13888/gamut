# C2PA (Coalition for Content Provenance and Authenticity) — content-credentials reference

Reference specifications for gamut's C2PA work, epic
[#239](https://github.com/visualcommons/gamut/issues/239).

**This directory has no single owning crate, and that makes it unusual here.** C2PA is a
cross-format payload, not a format: the same JUMBF manifest store is carried by an ISOBMFF `uuid`
box, a PNG chunk, a TIFF tag, a RIFF chunk and a run of JPEG APP11 segments. Ten crates consume
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
| [`C2PA_Specification_2.4.html`][spec-html] | 1,065,169 | canonical HTML; carries the clause anchors every gamut C2PA issue cites. **Text only** — see below |
| [`C2PA_Specification_2.4.pdf`][spec-pdf] | 8,139,117 | the complete rendition, figures included, paginated for stable citation (307 pages, "2.4, 2026-04-01") |
| [`C2PA_Schemas_2.4.zip`][spec-schemas] | 57,677 | the normative CDDL and JSON Schemas — 47 files: `cddl/*.cddl`, `c2pa_urn.abnf`, `valid_metadata_fields.yml`, the soft-binding OpenAPI and crJSON schemas |

[spec-html]: https://spec.c2pa.org/specifications/specifications/2.4/specs/C2PA_Specification.html
[spec-pdf]: https://spec.c2pa.org/specifications/specifications/2.4/specs/_attachments/C2PA_Specification.pdf
[spec-schemas]: https://spec.c2pa.org/specifications/specifications/2.4/specs/_attachments/C2PA_Schemas.zip

SHA-256, so a re-fetch or a silent upstream re-issue is detectable:

```
d55caebd96206f0de667962a4bab7098c6b6468fba68f8b55bcdd3a12d1ed26d  C2PA_Specification_2.4.html
a4e5142ef20e74271ac0cddef1a138fbbb77c756c2141abde1844e0c71f4d03a  C2PA_Specification_2.4.pdf
e3168d82cc995dc3eac5c3f2cebb88cd670f582544633d47df75aca16e9acfb4  C2PA_Schemas_2.4.zip
```

**The vendored HTML carries the prose and the clause anchors, not the figures.** It references 16
`_images/` assets, the Antora stylesheet and site JS, and renders five further diagrams from
`kroki.io` at view time; none of those are vendored. Two of the missing figures are cited by clauses
this epic depends on — Figure 4 (the JUMBF assertion box, §8.4.2.3) and Figure 8 (the Manifest
Store, §11.1.4.2). **Read any figure from the PDF**, which is self-contained. The site CSS/JS are
deliberately absent: the footer carves the Antora UI out of the CC BY grant as MPL-2.0, so it does
not belong in this tree.

The 2.4 index page's download menu only lists documents up to 2.3, but both `_attachments/` URLs
above resolve. Nothing in #239 consumes the CDDL yet — it is vendored because the JUMBF-interior
work deferred out of #239 is the only thing that ever will, and by then the 2.4 attachment may be
gone.

> **Extracting the schemas zip:** six of its 47 entries carry `../../../` path prefixes
> (`softbinding/partials/…`, `crJSON/partials/…`) reflecting the upstream site layout. Info-ZIP and
> Python's `zipfile` both strip the `../` components rather than escaping, so the effect is two
> stray `softbinding/` and `crJSON/` directories in the extraction target. Use `unzip -j`, or
> extract into a scratch directory.

### Attribution

Licence: **CC BY 4.0**, <https://creativecommons.org/licenses/by/4.0/>. All three files above are
vendored **verbatim and unmodified**; no derivative was made.

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
  needs **Annex D.2** (the JPEG APP11 field layout: `CI`/`En`/`Z`/`LBox`/`TBox`) and **clause 4**'s
  general box framing — in particular whether `LBox` counts its own 8-byte header, and the extended
  forms (`LBox == 1` ⇒ a 64-bit `XLBox` follows; `LBox == 0` ⇒ the box runs to the end of its
  container).
- **ISO/IEC 18477-3** — *JPEG XT box file format*. §A.3.1 defines the C2PA APP11 segment by
  reference to this document; it is the source of the marker-segment framing.
- **ISO/IEC 18181-2:2024** — *JPEG XL container* (`jumb` box, clause 9.3), cited by §A.3.9.

## Clause map — which crate consumes what

| crate | clause | what it takes from here |
| --- | --- | --- |
| `gamut-metadata` | §9.1, §11.1.4.2, §15.12.1.1, §9.2.6 | one hard binding per standard manifest; the store is a JUMBF superbox; the exclusion contract that makes a copied-forward manifest invalid |
| `gamut-heic` | §A.5.1–§A.5.3, §A.5.6, §15.12.2, §18.6 | top-level `uuid` box, user type `D8FEC3D6-…-C481`; `box_purpose` and the 8-byte merkle offset; BMFF excludes by box path, not byte offset |
| `gamut-isobmff` | §A.5.3 | placement: after `ftyp`, before the first `mdat` and before any `moov` |
| `gamut-avif` | §A.5 | same BMFF carriage; AVIF is named explicitly in §A.5.1 |
| `gamut-png` | §A.3.2, §18.5.4 | `caBX` chunk — ancillary, private, not-safe-to-copy; should precede `IDAT`. The chunk's `Length` and type bytes go *inside* the exclusion range |
| `gamut-dng` | §A.3.6, §18.5.5 | tag 52545 / `0xCD41`, type 7, in the **last** IFD of the main chain; the entry's `count` field *should* be a *second, disjoint* exclusion range |
| `gamut-tiff` | §A.3.6 | as DNG. §18.7.3.3 removed general box hash for TIFF, so `c2pa.hash.data` is the only binding |
| `gamut-riff` / `gamut-webp` | §A.3.7 | `C2PA` chunk, last sub-chunk of the first RIFF header chunk |
| `gamut-jpeg` | §A.3.1 | **descoped — see below** |

## Recorded decisions

### The JPEG APP11 slice is descoped until ISO/IEC 19566-5 is acquired

§A.3.1 defines the C2PA JPEG **field layout** entirely by reference to two paywalled documents:

> The C2PA Manifest Store shall be embedded as the data contained in an APP11 marker segment as
> defined in JPEG XT, ISO/IEC 18477-3. […] they shall be constructed as per the JPEG 1 standard and
> ISO 19566-5:2023, D.2. When writing multiple segments, they shall be written in sequential order,
> and they shall be contiguous (i.e., one segment immediately following the next).

The vendored text does give *some* normative APP11 requirements — the `FFEB` marker and the `Lp`
length (§18.5.3), contiguity and sequential order (above). What it does not give, anywhere, is the
**field layout inside** those segments: `CI`, `En`, `Z`, `LBox`, `TBox`. Annex D.2 is the only
source for that, and it is behind CHF 135. Writing the segmentation against
`c2pa-rs`'s implementation instead would violate `AGENTS.md`'s *"specification as source of
truth"*. **The JPEG slice of #239 is therefore out of scope until 19566-5 is purchased.**

**What this does and does not block.** PNG, DNG/TIFF and RIFF/WebP carriage is fully defined in the
vendored text, because in each the *host container* bounds the store: a PNG chunk `Length`, an IFD
entry `count`, a RIFF chunk size. Those slices need nothing from 19566-5.

BMFF is the one partial case, and it is partial in exactly one respect. Locating the box, checking
the user type, parsing `box_purpose` and skipping the 8-byte merkle offset are all fully specified
(§A.5.1–§A.5.3). But §A.5.3 permits *"zero or more unused padding bytes"* after the store, so the
box length does not bound the store — only the store's own outer JUMBF length does, and that field's
semantics live in 19566-5 clause 4. See the next section for exactly how far the incidental trace
here reaches and what a reader must therefore refuse.

### `En = 0x0211` is a `c2pa-rs` convention, not normative

`c2pa-rs` writes the JUMBF box instance number `En = 0x0211`, with the source comment *"can be any
unique ID, so we pick one that shouldn't conflict"* (recorded from #239; not checkable from this
tree). The value appears nowhere in the C2PA specification — that part is verified: `0x0211` and
`0211` do not occur in the vendored text.
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
the store. Reading that one length field is framing, not JUMBF interior, and is the only JUMBF value
any gamut crate reads under #239.

**The trace reaches exactly one sentence, and no further.** It says `LBox` is a 4-byte big-endian
unsigned integer. It does **not** say whether that count includes the 8-byte `LBox`+`TBox` header,
and it says nothing about the extended forms the JP2/JUMBF box family defines in 19566-5 clause 4
(`LBox == 1` ⇒ a 64-bit `XLBox` follows the `TBox`; `LBox == 0` ⇒ the box runs to the end of its
container). A reader that guesses either way can silently truncate or overrun a manifest store, and
because the store is opaque to gamut nothing downstream would catch it.

So a gamut crate reading this length **must refuse rather than guess**: treat any `LBox` below the
8-byte header, or larger than the bytes remaining in the enclosing box, as "not a manifest store"
and report absence — never an error, never a truncated payload. That covers `LBox ∈ {0, 1}` by
construction. Should a container ever legitimately need the extended forms, 19566-5 must be acquired
first; that is the same procurement gate as the JPEG slice, on a narrower question.

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

Three properties of `c2pa-rs` **0.90.16** shaped how gamut splits the work. They are recorded from
[#239](https://github.com/visualcommons/gamut/issues/239) and are **not checkable from this tree** —
no `c2pa` dependency, submodule or `tooling/` crate exists yet. Re-confirm them against the pinned
version when `tooling/c2pa-oracle` lands, and pin that version in its `Cargo.toml` so a later
`c2pa-rs` cannot invalidate this rationale silently.

- `Builder`'s `*_embeddable` path plus `composed_manifest(bytes, "application/c2pa")` yields raw
  manifest-store bytes for a host to embed itself, and `CallbackSigner` carries a `reserve_size` —
  it already models signing as reserve-then-fill, which is the seam gamut exposes.
- There is **no cheap parse-only mode**: `ValidationState::Invalid` is also what you get when
  verification is disabled, so `c2pa-rs` cannot serve as a "parse but don't judge" front end. That
  is why gamut owns the locate-and-bound step rather than shelling out for it.
- Do **not** build on `c2pa::jumbf_io` — its public functions name crate-private traits
  (`CAIRead`/`CAIReadWrite`), a private-in-public leak.
