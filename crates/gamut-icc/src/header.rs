//! The 128-byte ICC profile header.

/// The fixed 128-byte header that opens every ICC profile (ICC.1:2022 §7.2).
///
/// The header records the profile's size, the device/connection color spaces it relates, the
/// profile version, the default rendering intent, and an MD5 identifier. Only the most
/// load-bearing fields are modelled here; the reserved and date/platform fields are added in the
/// implementation phase.
pub struct ProfileHeader {
    /// Total profile size in bytes (`size` field).
    pub size: u32,
    /// Preferred CMM signature (e.g. `'appl'`, `'lcms'`), or zero.
    pub preferred_cmm: u32,
    /// Profile format version (the `major.minor.bugfix` of the spec it conforms to).
    pub version: ProfileVersion,
    /// What kind of device or transform the profile describes.
    pub device_class: DeviceClass,
    /// The device color space the profile's data side uses (the `A` side).
    pub data_color_space: ColorSpace,
    /// The profile connection space (the `B` side) — always `XYZ` or `Lab`.
    pub pcs: ColorSpace,
    /// The default rendering intent.
    pub rendering_intent: RenderingIntent,
    /// Profile creator signature, or zero.
    pub creator: u32,
    /// The 16-byte profile ID (an MD5 of the profile with certain fields zeroed), or all-zero if
    /// unset.
    pub profile_id: [u8; 16],
}

/// An ICC profile format version, e.g. 4.4.0 or 2.4.0.
pub struct ProfileVersion {
    /// Major version (`2` or `4` in practice).
    pub major: u8,
    /// Minor version (the high nibble of the second version byte).
    pub minor: u8,
    /// Bug-fix version (the low nibble of the second version byte).
    pub bugfix: u8,
}

/// The profile/device class, stored in the header's `deviceClass` field (ICC.1:2022 §7.2.5).
pub enum DeviceClass {
    /// `'scnr'` — input device (scanner, camera).
    Input,
    /// `'mntr'` — display device (monitor).
    Display,
    /// `'prtr'` — output device (printer).
    Output,
    /// `'link'` — a device link (a fused device-to-device transform).
    DeviceLink,
    /// `'spac'` — a color-space conversion profile.
    ColorSpace,
    /// `'abst'` — an abstract profile (color-space to color-space, not device-bound).
    Abstract,
    /// `'nmcl'` — a named-color profile.
    NamedColor,
}

/// A color space signature, used for both the data color space and the profile connection space
/// (ICC.1:2022 §7.2.6–7.2.7). Only the common spaces are listed; the multi-channel `nCLR` spaces
/// are added in the implementation phase.
pub enum ColorSpace {
    /// `'XYZ '` — CIE XYZ (a valid PCS).
    Xyz,
    /// `'Lab '` — CIE L\*a\*b\* (a valid PCS).
    Lab,
    /// `'Luv '` — CIE L\*u\*v\*.
    Luv,
    /// `'YCbr'` — YCbCr.
    YCbCr,
    /// `'Yxy '` — CIE Yxy.
    Yxy,
    /// `'RGB '` — RGB.
    Rgb,
    /// `'GRAY'` — grayscale.
    Gray,
    /// `'HSV '` — HSV.
    Hsv,
    /// `'HLS '` — HLS.
    Hls,
    /// `'CMYK'` — CMYK.
    Cmyk,
    /// `'CMY '` — CMY.
    Cmy,
}

/// The rendering intent, stored in the header's `renderingIntent` field (ICC.1:2022 §7.2.15).
pub enum RenderingIntent {
    /// `0` — perceptual.
    Perceptual,
    /// `1` — media-relative colorimetric.
    MediaRelativeColorimetric,
    /// `2` — saturation.
    Saturation,
    /// `3` — ICC-absolute colorimetric.
    IccAbsoluteColorimetric,
}
