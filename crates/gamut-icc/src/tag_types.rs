//! The element types a tag's data can take.

/// The type of a tag's element data, identified by the four-byte type signature at the start of the
/// element (ICC.1:2022 §10). Representative subset; the full set is filled in during implementation.
///
/// The **keystone** of the crate is the multi-dimensional transform types — `lutAToB`/`lutBToA`
/// (`mAB `/`mBA `) and the legacy `lut8`/`lut16` — which carry the matrix/curve/CLUT pipeline that
/// drives device↔PCS conversion.
pub enum TagType {
    /// `XYZ ` — one or more CIE XYZ triplets (`XYZType`).
    Xyz,
    /// `curv` — a one-dimensional tone curve, sampled or an identity/gamma (`curveType`).
    Curve,
    /// `para` — a parametric tone curve (`parametricCurveType`).
    ParametricCurve,
    /// `text` — 7-bit ASCII text (`textType`).
    Text,
    /// `mluc` — language-tagged Unicode text (`multiLocalizedUnicodeType`).
    MultiLocalizedUnicode,
    /// `sig ` — a four-byte signature value (`signatureType`).
    Signature,
    /// `dtim` — a date-time (`dateTimeType`).
    DateTime,
    /// `sf32` — an array of s15Fixed16 numbers (`s15Fixed16ArrayType`, e.g. `chad`).
    S15Fixed16Array,
    /// `mft1` — the legacy 8-bit lookup transform (`lut8Type`).
    Lut8,
    /// `mft2` — the legacy 16-bit lookup transform (`lut16Type`).
    Lut16,
    /// `mAB ` — the device-to-PCS transform (`lutAToBType`: A-curves, CLUT, M-curves, matrix,
    /// B-curves).
    LutAToB,
    /// `mBA ` — the PCS-to-device transform (`lutBToAType`).
    LutBToA,
    /// `ncl2` — a named-color list (`namedColor2Type`).
    NamedColor2,
}
