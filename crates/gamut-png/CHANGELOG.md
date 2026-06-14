# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/justin13888/gamut/releases/tag/gamut-png-v0.1.0) - 2026-06-14

### Added

- *(png)* wire PNG into the umbrella crate and the CLI
- *(png)* add lossless reductions and brute-force filter selection
- *(png)* embed EXIF, ICC profiles, and XMP metadata
- *(png)* add standard ancillary chunks (colour, physical, timing, text)
- *(png)* add sub-byte bit depths (1-bit bilevel, 1/2/4-bit indexed)
- *(png)* add indexed colour with PLTE and palette transparency
- *(png)* support all non-indexed colour types at 8 and 16 bits
- *(png)* add the five scanline filters with MinSumAbs selection
- *(png)* scaffold gamut-png with RGB8 keystone + libpng oracle
