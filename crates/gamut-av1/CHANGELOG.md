# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1](https://github.com/justin13888/gamut/compare/gamut-av1-v0.2.0...gamut-av1-v0.2.1) - 2026-06-09

### Added

- *(av1)* complete directional intra modes (D45/D67/D203) [lossy-intra P12b]
- *(av1)* directional intra modes V/H/D135/D113/D157 [lossy-intra P12a]
- *(av1)* non-directional luma intra modes (PAETH/SMOOTH) [lossy-intra P11]
- *(av1)* per-block transform-type selection from TX_SET_INTRA_2
- *(av1)* extend lossy quantizer range to all four CDF contexts
- *(av1)* lossy intra reconstruction pivot (DCT + quant), bit-exact
- *(av1)* add 2-D transform assembly (inverse + forward)
- *(av1)* add quantizer tables, dequant, and encoder quantizer

### Other

- clarify av1 codec vs avif format distinction
- Merge pull request #20 from justin13888/docs/crate-readmes
- add structurally consistent README to every crate
