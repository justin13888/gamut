This directory contains the official DNG specification (the latest is 1.71.0 as of June 2026) along with a downloaded copy of Adobe DNG SDK as a zip.

As of writing, the latest corresponding DNG SDK from Adobe (that can be used as reference implementation) is v1.7.1 Build 2611 (June 9, 2026) ([download link](https://www.adobe.com/go/dng_sdk)).

Real camera DNGs are **not** kept here. Adobe's SDK ZIP above ships its own `sample_files/`, which
are Adobe-authored; files that real cameras wrote live in the separate `gamut-dng-samples`
submodule at `third_party/gamut-dng-samples` (CC0, from raw.pixls.us) and back the conformance
tier in `tooling/gamut-dng-real-conformance` — see issue #174 and `crates/gamut-dng/STATUS.md`.
Fetch it with `mise run fetch-dng-samples`; it is marked `update = none` in `.gitmodules`, so a
normal `git submodule update --init --recursive` does not pull its ~178 MiB.
