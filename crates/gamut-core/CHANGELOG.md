# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1](https://github.com/justin13888/gamut/compare/gamut-core-v0.2.0...gamut-core-v0.2.1) - 2026-06-14

### Added

- *(core)* add shared HDR/SDR luminance reference constants
- *(core)* add GrayAlpha8/GrayAlpha16 pixel markers

### Other

- Merge branch 'master' into feat/gamut-tonemap-v1-188
- Merge branch 'master' into feat/png

## [0.2.0](https://github.com/justin13888/gamut/compare/gamut-core-v0.1.0...gamut-core-v0.2.0) - 2026-06-12

### Added

- *(core)* add EncodeImage/DecodeImage traits alongside Encoder/Decoder
- *(core)* add ImageRef/ImageBuf branded image buffers
- *(core)* add Pixel/Sample/ColorModel pixel vocabulary
- *(core)* add validated Dimensions constructor and area helpers

### Other

- *(core)* [**breaking**] remove the legacy Encoder/Decoder traits
- Merge pull request #20 from justin13888/docs/crate-readmes
- add structurally consistent README to every crate
