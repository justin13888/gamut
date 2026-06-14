# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0](https://github.com/justin13888/gamut/compare/gamut-color-v0.3.0...gamut-color-v0.4.0) - 2026-06-14

### Other

- Merge branch 'master' into feat/gamut-tonemap-v1-188
- *(color)* [**breaking**] source luminance constants from gamut-core, rename SDR->HDR reference white
- Merge pull request #151 from justin13888/feat/benchmarks
- *(color)* close mutation-testing gaps

## [0.3.0](https://github.com/justin13888/gamut/compare/gamut-color-v0.2.0...gamut-color-v0.3.0) - 2026-06-12

### Added

- *(av1)* [**breaking**] type ReconImage.bit_depth as BitDepth; add Planar8 view ctor
- *(color)* add clip_pixel8 pixel-saturation helper
- *(av1)* superres — horizontal upscaling (§7.16) with loop restoration after upscale
- *(color)* BT.601 YCbCr 4:2:0 conversion for VP8

### Other

- *(color)* [**breaking**] delete the unused PixelFormat enum, document BitDepth/ChromaSubsampling
- Merge pull request #142 from justin13888/feat/avif-still-image-compliance
- *(av1)* [**breaking**] widen reconstruction to u16 for high-bit-depth support
- *(color)* use Ord::clamp in clip_pixel8
- Merge pull request #101 from justin13888/feat/av1-lossy-superres
- Merge pull request #20 from justin13888/docs/crate-readmes
- add structurally consistent README to every crate
