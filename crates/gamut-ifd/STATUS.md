# gamut-ifd — TIFF/IFD container core implementation status

Part of the **image metadata primitives** campaign (GitHub issue #34). `gamut-ifd` factors the
TIFF Image File Directory structure (`references/tiff/tiff6.pdf` §2, plus `references/exif` for the
EXIF profile) out as a shared primitive consumed by [`gamut-exif`](../gamut-exif). Delivered as a
stack of small, individually-reviewable PRs onto the `feat/metadata-primitives` integration branch;
each PR is independently green (`just test`/`lint`/`format-check`/`coverage` ≥ 80%).

**Keystone:** the **two-pass offset layout** in the writer. Out-of-line values and following IFDs
need absolute offsets that are only known after sizes are fixed, so the writer must plan the layout
then back-patch offset words; a read → write → read round-trip must reproduce the directory exactly.
Once that spine is solid, the reader and the typed-value paths are mechanical.

**Second consumer (out of scope):** [`gamut-tiff`](../gamut-tiff) (issue #107) currently ships its
own self-contained IFD reader on the `feat/tiff` branch. The structural type names here mirror it so
that, once both land on `master`, a follow-up can refactor `gamut-tiff` onto `gamut-ifd` (delete the
inlined copy, `use gamut_ifd::…`) with minimal diff. That refactor is tracked separately and is
**not** part of this campaign.

**Oracle:** differential vs **libtiff** / **exiv2** (dev-only FFI) — the IFD structure a written
stream produces must match what the reference libraries parse, and vice versa.

## Phases

| Phase | Spec § | Scope | Status |
| ----- | ------ | ----- | ------ |
| P1 | — | Scaffold: crate, workspace wiring, docs, region-free data-model skeleton | ✅ in progress |
| P2 | §2 | Header + single-IFD reader: II/MM byte order, magic, entry decode for all 12 field types | ☐ |
| P3 | §2 | Value resolution: inline (≤ 4 bytes) vs out-of-line offsets; multi-IFD chains (`next` links) | ☐ |
| P4 | §2 | **Keystone** — writer with two-pass offset layout + back-patching; read→write→read round-trip | ☐ |
| P5 | §2 | Sub-IFD pointers + nested directories (the Exif/GPS/Interop offset-tag pattern EXIF needs) | ☐ |
| P6 | §2 | Robustness: offset-loop / overlap / truncation hardening + fuzz corpus | ☐ |
| P7 | — | libtiff/exiv2 differential oracle gate | ☐ |
| P8 | — | BigTIFF (8-byte offsets/counts, `Long8`/`SLong8`/`Ifd8`) — gated `bigtiff` feature, additive | ⊘ deferred |
