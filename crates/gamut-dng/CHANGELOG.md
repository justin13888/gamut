# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/justin13888/gamut/releases/tag/gamut-dng-v0.1.0) - 2026-06-14

### Added

- *(gamut-dng)* embed + decode EXIF/XMP/IPTC/ICC metadata
- *(gamut-dng)* lossless JPEG (SOF3) encode + decode
- *(gamut-dng)* Deflate/ZIP compression (encode + decode)
- *(gamut-dng)* BigTIFF (64-bit) DNG support
- *(gamut-dng)* full DNG decoder
- *(gamut-dng)* bit-depth packing (8/10/12/14/16) + default crop
- *(gamut-dng)* full colour-calibration profile
- *(gamut-dng)* encode LinearRaw (demosaiced) images
- *(gamut-dng)* encode uncompressed CFA DNG (keystone)
- *(gamut-dng)* add DNG tag and value tables
- *(gamut-dng)* scaffold DNG codec crate

### Other

- *(gamut-dng)* use an odd width in the linear round-trip
- *(gamut-dng)* close remaining DNG codec mutation gaps
- *(gamut-dng)* close lossless-JPEG codec mutation gaps
- *(gamut-dng)* cover the 8-bit bitpack fast path
- *(gamut-dng)* clarify DNGVersion octets and Deflate codec choice
- *(gamut-dng)* reuse gamut-bitstream sample packing
- *(gamut-dng)* finalize STATUS, README, and workspace layout
- *(gamut-dng)* gate CFA DNG output on the Adobe SDK + libtiff
