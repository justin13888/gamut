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

/// A display mirror applied by a reader (the `imir` property, ISO/IEC 23008-12 §6.5.12). The stored
/// pixels are unchanged. Variants are named by the visual effect rather than the spec's mirror
/// *axis*, which is the usual source of confusion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mirror {
    /// Mirror left↔right (about a vertical axis; `imir` `axis = 0`).
    LeftRight,
    /// Mirror top↔bottom (about a horizontal axis; `imir` `axis = 1`).
    TopBottom,
}

impl Mirror {
    /// The `imir` `axis` field: `0` for [`Mirror::LeftRight`], `1` for [`Mirror::TopBottom`].
    pub(crate) fn axis(self) -> u8 {
        match self {
            Mirror::LeftRight => 0,
            Mirror::TopBottom => 1,
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
        assert_eq!(Mirror::LeftRight.axis(), 0);
        assert_eq!(Mirror::TopBottom.axis(), 1);
    }
}
