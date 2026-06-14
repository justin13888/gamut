# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1](https://github.com/justin13888/gamut/compare/gamut-tiff-v0.2.0...gamut-tiff-v0.2.1) - 2026-06-14

### Other

- Merge pull request #151 from justin13888/feat/benchmarks
- *(tiff)* close mutation-testing gaps

## [0.2.0](https://github.com/justin13888/gamut/compare/gamut-tiff-v0.1.0...gamut-tiff-v0.2.0) - 2026-06-12

### Added

- *(tiff)* [**breaking**] migrate to typed EncodeImage/DecodeImage, drop weakly-typed methods

### Other

- *(core)* [**breaking**] remove the legacy Encoder/Decoder traits
- *(tiff)* type the palette colour table as Palette8
