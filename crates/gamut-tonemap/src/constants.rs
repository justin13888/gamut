//! Reference luminance levels and per-operator parameter defaults.
//!
//! Values are expressed either as normalized linear units (where display white is `1.0`) or in
//! candela per square metre (cd/m², "nits") where an absolute level matters, so callers and
//! operators share one definition of common HDR/SDR reference points.

/// Normalized SDR display white in linear units (`1.0`). Default upper bound for
/// [`Clamp`](crate::operators::Clamp).
pub const SDR_WHITE_NORMALIZED: f32 = 1.0;

/// SDR diffuse-white luminance in cd/m² (nits): the classic 100-nit reference.
pub const SDR_REFERENCE_WHITE_NITS: f32 = 100.0;

/// HDR reference (graphics / diffuse) white in cd/m² (nits): 203, per ITU-R BT.2408.
pub const HDR_REFERENCE_WHITE_NITS: f32 = 203.0;

/// PQ peak luminance in cd/m² (nits): the 10 000-nit maximum of SMPTE ST 2084.
pub const PQ_PEAK_NITS: f32 = 10_000.0;

/// Default white point for [`ReinhardExtended`](crate::operators::ReinhardExtended) when none is
/// given: the HDR-to-SDR reference-white ratio (`203 / 100 ≈ 2.03`).
pub const DEFAULT_REINHARD_WHITE: f32 = HDR_REFERENCE_WHITE_NITS / SDR_REFERENCE_WHITE_NITS;
