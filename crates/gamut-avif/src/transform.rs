//! Display-orientation transforms (`irot`/`imir` item properties). A reader applies them at display
//! time — the stored pixels are unchanged — so they record e.g. a camera's EXIF orientation without
//! re-encoding rotated samples.

/// A display rotation applied **anti-clockwise** by a reader (the `irot` property, ISO/IEC 23008-12
/// §6.5.10). The stored pixels are unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Rotation {
    /// No rotation (the default).
    #[default]
    None,
    /// 90° anti-clockwise.
    Ccw90,
    /// 180°.
    Ccw180,
    /// 270° anti-clockwise (= 90° clockwise).
    Ccw270,
}

impl Rotation {
    /// The `irot` `angle` field: the rotation in 90° anti-clockwise steps (`0..=3`).
    pub(crate) fn quarter_turns(self) -> u8 {
        match self {
            Rotation::None => 0,
            Rotation::Ccw90 => 1,
            Rotation::Ccw180 => 2,
            Rotation::Ccw270 => 3,
        }
    }
}

/// A display mirror applied by a reader (the `imir` property, ISO/IEC 23008-12:2022 §6.5.12). The
/// stored pixels are unchanged. Variants are named by the visual effect rather than the spec's
/// mirror *axis*, which is the usual source of confusion: per the 2022 text (the semantics
/// libheif and libavif implement), `axis = 0` exchanges the top and bottom parts and `axis = 1`
/// exchanges the left and right parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mirror {
    /// Mirror left↔right (`imir` `axis = 1`).
    LeftRight,
    /// Mirror top↔bottom (`imir` `axis = 0`).
    TopBottom,
}

impl Mirror {
    /// The `imir` `axis` field: `1` for [`Mirror::LeftRight`], `0` for [`Mirror::TopBottom`]
    /// (ISO/IEC 23008-12:2022 §6.5.12).
    pub(crate) fn axis(self) -> u8 {
        match self {
            Mirror::LeftRight => 1,
            Mirror::TopBottom => 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_quarter_turns() {
        assert_eq!(Rotation::None.quarter_turns(), 0);
        assert_eq!(Rotation::Ccw90.quarter_turns(), 1);
        assert_eq!(Rotation::Ccw180.quarter_turns(), 2);
        assert_eq!(Rotation::Ccw270.quarter_turns(), 3);
        assert_eq!(Rotation::default(), Rotation::None);
    }

    #[test]
    fn mirror_axis() {
        // ISO/IEC 23008-12:2022 §6.5.12: axis 0 exchanges top/bottom, axis 1 exchanges
        // left/right.
        assert_eq!(Mirror::LeftRight.axis(), 1);
        assert_eq!(Mirror::TopBottom.axis(), 0);
    }
}
