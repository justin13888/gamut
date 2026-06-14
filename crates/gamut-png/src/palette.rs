//! Palette (PLTE) and palette transparency (tRNS) for indexed-colour PNG (PNG spec §11.2.2/§11.3.2).

use gamut_core::{Error, Result};

/// A PNG palette: 1–256 RGB entries, with optional per-entry alpha (written as a tRNS chunk).
///
/// Entries without an alpha value are fully opaque. Indexed images reference entries by index.
#[derive(Debug, Clone)]
pub struct PngPalette {
    rgb: Vec<[u8; 3]>,
    /// Per-entry alpha for the leading entries; entries beyond `alpha.len()` are opaque.
    alpha: Vec<u8>,
}

impl PngPalette {
    /// Builds an opaque palette from 1–256 RGB entries.
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if there are zero or more than 256 entries.
    pub fn new(entries: &[[u8; 3]]) -> Result<Self> {
        Self::with_transparency(entries, &[])
    }

    /// Builds a palette with per-entry transparency. `alpha[i]` is the alpha of palette entry `i`;
    /// `alpha` may be shorter than `rgb` (the remaining entries are opaque).
    ///
    /// # Errors
    ///
    /// Returns [`Error::InvalidInput`] if there are zero or more than 256 RGB entries, or if there
    /// are more alpha values than RGB entries.
    pub fn with_transparency(rgb: &[[u8; 3]], alpha: &[u8]) -> Result<Self> {
        if rgb.is_empty() || rgb.len() > 256 {
            return Err(Error::InvalidInput(
                "PNG: palette must have 1..=256 entries",
            ));
        }
        if alpha.len() > rgb.len() {
            return Err(Error::InvalidInput(
                "PNG: more tRNS entries than palette entries",
            ));
        }
        Ok(Self {
            rgb: rgb.to_vec(),
            alpha: alpha.to_vec(),
        })
    }

    /// The number of palette entries (1–256).
    #[must_use]
    pub fn len(&self) -> usize {
        self.rgb.len()
    }

    /// Always `false` — a palette has at least one entry (kept for API completeness).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rgb.is_empty()
    }

    /// The PLTE chunk payload: RGB triples, flattened.
    pub(crate) fn plte(&self) -> Vec<u8> {
        self.rgb.iter().flatten().copied().collect()
    }

    /// The tRNS chunk payload (the alpha values), or `None` if the palette is fully opaque.
    pub(crate) fn trns(&self) -> Option<&[u8]> {
        if self.alpha.is_empty() {
            None
        } else {
            Some(&self.alpha)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_invalid_sizes() {
        assert!(PngPalette::new(&[]).is_err());
        assert!(PngPalette::new(&vec![[0, 0, 0]; 257]).is_err());
        assert!(PngPalette::with_transparency(&[[0, 0, 0]], &[1, 2]).is_err());
        assert!(PngPalette::new(&[[1, 2, 3]]).is_ok());
    }

    #[test]
    fn serialises_plte_and_trns() {
        let p = PngPalette::with_transparency(&[[1, 2, 3], [4, 5, 6]], &[0]).unwrap();
        assert_eq!(p.len(), 2);
        assert_eq!(p.plte(), vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(p.trns(), Some(&[0u8][..]));
        let opaque = PngPalette::new(&[[7, 8, 9]]).unwrap();
        assert_eq!(opaque.trns(), None);
    }
}
