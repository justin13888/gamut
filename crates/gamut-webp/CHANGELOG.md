# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.1](https://github.com/justin13888/gamut/compare/gamut-webp-v0.1.0...gamut-webp-v0.1.1) - 2026-06-09

### Added

- *(webp)* emit multi-group entropy image (meta prefix codes)
- *(webp)* emit LZ77 backward references and color cache
- *(webp)* emit color-indexing (palette) transform
- *(webp)* emit subtract-green, predictor, and color transforms
- *(webp)* implement VP8L encoder (literals + single prefix-code group)
- *(webp)* implement full VP8L lossless decoder
- *(webp)* implement VP8L LZ77 distance mapping and color cache
- *(webp)* implement VP8L inverse transforms (subtract-green, predictor, color, color-indexing)
- *(webp)* implement canonical prefix codes (decode + length-limited encode)
- *(webp)* implement VP8L header read/write
- *(webp)* implement LSB-first VP8L bit reader and writer
- *(webp)* scaffold codec module tree and wire encoder/decoder API

### Other

- *(webp)* mark VP8L lossless (M0/M1) implemented across STATUS, docs, and READMEs
- *(webp)* expand libwebp oracle matrix and add decoder robustness corpus
- *(webp)* document scope decisions for non-core features
- *(webp)* add libwebp differential oracle harness
- *(webp)* add two-part implementation STATUS.md and refresh READMEs
- clarify image-first crate boundaries
- Merge pull request #20 from justin13888/docs/crate-readmes
- add structurally consistent README to every crate
