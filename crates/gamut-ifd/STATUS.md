# gamut-ifd — TIFF/IFD container core implementation status

`gamut-ifd` factors the TIFF Image File Directory structure (`references/tiff/tiff6.pdf` §2, plus
`references/tiff/bigtiff.html` for BigTIFF) out as a shared primitive consumed by both
[`gamut-tiff`](../gamut-tiff) (the TIFF codec, issue #107) and [`gamut-exif`](../gamut-exif) (EXIF
metadata, issue #34). It models *structure only* — byte order, field types, values, the IFD chain,
and the offset-driven read/write spine — never pixels, compression, or photometry.

**Keystone:** the **two-pass offset layout** in the writer ([`write`](src/writer.rs)). Out-of-line
values and following IFDs need absolute offsets that are only known after sizes are fixed, so the
writer plans the layout then back-patches the offset words; a read → write → read round-trip
reproduces the directory exactly. At v1 the symmetry extends to whole sub-IFD trees:
`read_tree(&write(&file)?, tags)? == file`.

## How it was built

The structural core was migrated from `gamut-tiff`'s self-contained IFD implementation (issue #107):
`gamut-tiff` was developed first with an inlined IFD reader/writer, and the type names here were
authored to mirror it, so the move was near-zero-diff. `gamut-tiff` now consumes this crate (with
the `bigtiff` feature) instead of its own copy; its libtiff differential oracle exercises these exact
read/write code paths byte-for-byte.

## Phases

| Phase | Spec § | Scope | Status |
| ----- | ------ | ----- | ------ |
| P1 | — | Scaffold: crate, workspace wiring, docs, region-free data-model skeleton | ✅ done |
| P2 | §2 | Header + single-IFD reader: II/MM byte order, magic, entry decode for all 12 field types | ✅ done |
| P3 | §2 | Value resolution: inline (≤ offset width) vs out-of-line offsets; multi-IFD chains (`next` links) | ✅ done |
| P4 | §2 | **Keystone** — writer with two-pass offset layout + back-patching; read→write→read round-trip | ✅ done |
| P5 | §2 | Sub-IFD pointers + nested directories (the SubIFDs/Exif/GPS offset-tag pattern) | ✅ done (#109 write, #181 read) |
| P6 | §2 | Robustness: offset-loop / overlap / truncation guards + malformed-input corpus | ✅ done (#181) |
| P7 | — | libtiff/exiv2 differential oracle gate (via the consuming codecs — see below) | ✅ via codecs |
| P8 | — | BigTIFF (8-byte offsets/counts, `Long8`/`SLong8`/`Ifd8`) — gated `bigtiff` feature, additive | ✅ done |
| P9 | §2 | RAW-grade streaming: `ReadAt` sources (slice / `Read + Seek` / rebased), lazy `IfdReader`, structural `tags` | ✅ done (#252) |

P5's **write** side landed with the DNG codec (issue #109): [`write`](src/writer.rs) lays out the
whole IFD *tree* — [`Ifd::set_sub_ifd`](src/entry.rs) attaches children under a pointer tag
(`SubIFDs`/`ExifIFD`/…), the writer places them and synthesises the offset-array field. The **read**
side closed with v1 (issue #181): [`read_tree`](src/reader.rs) follows caller-named pointer tags
(the generic [`read`] cannot know which `LONG` tags are offsets) with depth and cycle guards,
rebuilding the tree `write` flattens; [`read_ifd_at`](src/reader.rs) stays as the per-pointer escape
hatch for lenient decoders (gamut-dng skips unparseable children while hunting the raw IFD,
gamut-exif tolerates malformed sub-IFDs in lenient mode).

P6's corpus ([`tests/robustness.rs`](tests/robustness.rs)) drives `read`/`read_tree` over specific
malformed inputs, a full truncation sweep, an LCG byte-flip fuzz, and an exhaustive single-byte
overwrite sweep, on both variants — and immediately caught (and fixed) a hostile-BigTIFF entry-count
multiply overflow.

P7 is satisfied **through the consuming codecs**, deliberately: `gamut-tiff`'s libtiff oracle
round-trips real TIFF containers byte-for-byte through this crate's reader/writer (including
BigTIFF, multi-IFD, and sub-IFD structure), and `gamut-exif`'s exiv2 oracle parses/round-trips bare
TIFF streams through the same paths (the 0th IFD and the Exif sub-IFD behind the pointer). A direct
oracle dev-dependency here would drag the C-oracle toolchain into this crate's test cycle without
exercising any path the codec gates do not already cover byte-exactly.

## v1 surface (issue #181)

The API was frozen after a full-surface review; the additions and breaks:

- **Spec fix** — multi-string ASCII/UTF-8 values (TIFF 6.0 §2) round-trip: decode strips exactly
  the terminating NUL and preserves interior separators.
- **Fallible `write`** — classic-width overflow (entry count > `u16::MAX`, layout > 4 GiB) is a
  typed error instead of silent truncation; the layout contract (word alignment, structural
  determinism) is documented API.
- **`read_tree`** — write's inverse over sub-IFD trees; sub-IFD groups are tag-sorted (canonical).
- **`Ifd::remove`**, **`Value::as_str`/`as_bytes`/`as_rationals`/`as_srationals`**,
  **`Value::offset_array`**, **`align_word`** — the pieces every consumer had hand-rolled.
- **Single canonical paths** — modules are private; the surface is the crate-root re-export list
  (plus, since P9, the [`tags`](src/tags.rs) module — the one *named* module, holding the
  structural pointer-tag constants).

## P9 — RAW-grade streaming (issue #252)

Downstream RAW decoding (rawshift's ARW/CR2/NEF/DNG engines) parses multi-hundred-MB camera
files whose IFD structure is kilobytes; loading the file to hand `read` a slice is the wrong
shape. P9 adds the streaming layer **additively** — the frozen v1 slice surface is untouched:

- [`ReadAt`](src/source.rs) — positioned exact reads + length; implemented by `&[u8]`,
  `StreamSource<R: Read + Seek>`, and `Rebased<S>` (the offset-shifting adapter maker-note
  mini-IFDs and embedded TIFF blocks need). Transport failures surface as the new
  `gamut_core::Error::Io`; a stream that ends early stays `InvalidInput`, like a short slice.
- [`IfdReader`](src/stream.rs) — lazy walker: `read_ifd` fetches one directory body into raw
  entries (value/offset words verbatim, in file byte order); `value` fetches/decodes one value
  on demand, span-checked against the source length *before* allocating; `ifds()` iterates the
  chain; `read_file` / `read_tree` / the coverage methods mirror the slice APIs exactly.
- [`tags`](src/tags.rs) — the structural pointer tags (`SubIFDs`, `ExifIFD`, `GPSInfo`,
  `InteroperabilityIFD`; `MakerNote` as the named-but-not-followable blob carrier), deduplicating
  the three consumers' copies. The one sanctioned exception to "no tag semantics": these tags
  name the directory graph itself.

The guard story: the chain loop/length guard (`ChainGuard`) and the sub-IFD depth/cycle guards
(`resolve_pointers_with`) are *shared* with the slice path; the directory-body walk is mirrored
(the data flows differ — decode-in-loop vs raw capture), and `tests/robustness.rs` runs the whole
hostile corpus through both paths asserting **agreement**, so a mutant in either copy dies. Both
paths are u64-native end to end (no `as usize` truncation of hostile 64-bit widths) — the
streaming path from birth, the slice path since the #262 audit below.
Layout requirement 3 of #252 (deterministic write offsets) was already the P4 keystone contract;
`tests/streaming.rs` pins the rest of the capability surface (three source shapes × orders ×
variants, coverage parity, the ≤64-read-bytes laziness contract, the maker-note pattern).

## Hardening audit (issue #262)

Rawshift's migration off its hardened binrw TIFF parser required a parity audit of this crate
against hostile camera TIFFs — its `ParseError` case list is the acceptance checklist. Verdicts:

| Category | Verdict |
| -------- | ------- |
| Magic / byte-order validation | Guarded in the shared `read_header`; messages pinned |
| IFD / value offset bounds | Guarded on both paths, with the two-error offset-vs-span distinction |
| Circular IFDs | Guarded by the shared `ChainGuard` (chain loop + 65 536 cap) and `resolve_pointers_with` (pointer loop + depth 16); both loop shapes pinned |
| Offset-arithmetic overflow | Checked u64 arithmetic on both paths — the audit's one real finding, fixed below |
| Truncated files | Typed error at every stage (header / count / body / value); full prefix sweep |
| Overlapping records | **Report-not-reject, by design**: TIFF legitimately lets structures share storage, so the parse succeeds and the opt-in `Coverage` accounting surfaces `CoverageReport.overlaps` — pinned by an adversarial header-overlap fixture |

**The finding:** the slice reader cast u64 counts/offsets to `usize` *before* its checked
arithmetic — on a 32-bit target with `bigtiff`, a hostile 64-bit width truncated into a silent
in-bounds misparse instead of an error. The reader now stays u64 until a bound against the data
length proves each conversion lossless, mirroring the streaming path. The guard is verified by
construction on 64-bit CI (where `u64 → usize` cannot truncate, so no new branch is dead there);
32-bit CI (`check-cross`) is compile-only.

**The acceptance artifact:** [`tests/hardening_audit.rs`](tests/hardening_audit.rs) pins the
exact `Error::InvalidInput` string per checklist case — on both paths, including the two cases
where the data flows legitimately phrase a failure differently (a directory offset at/past EOF:
slice bounds-check vs streaming positioned read). Those strings are rawshift's mapping contract.

Assumptions: rawshift's malformed-fixture corpus was not yet contributed, so synthetic in-repo
fixtures stand in (the corpus can slot into `tests/robustness.rs` when it lands); and per the
issue ("correctness verification, not an API ask") error granularity stays
`InvalidInput(&'static str)` — no per-case error variants were added.
