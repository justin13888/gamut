# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0](https://github.com/justin13888/gamut/compare/gamut-tonemap-v0.1.2...gamut-tonemap-v0.2.0) - 2026-06-14

### Added

- *(tonemap)* re-export operators at the crate root
- *(tonemap)* add ACES, Hable, Drago, and exposure operators

### Other

- *(tonemap)* add STATUS.md and refresh the README for v1
- *(tonemap)* cross-check Reinhard against gamut-color and harden operator tests
- *(tonemap)* [**breaking**] source luminance constants from gamut-core, drop local duplicates
- Merge pull request #151 from justin13888/feat/benchmarks
- *(tonemap)* close mutation-testing gaps

## [0.1.2](https://github.com/justin13888/gamut/compare/gamut-tonemap-v0.1.1...gamut-tonemap-v0.1.2) - 2026-06-12

### Other

- updated the following local packages: gamut-core
