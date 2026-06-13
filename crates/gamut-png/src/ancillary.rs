//! Standard ancillary chunks (PNG spec §11.3): colour-space, physical, timing, and text metadata.
//!
//! These are optional. The encoder accumulates whatever the caller sets and emits the chunks in the
//! order PNG requires (Table 7): colour-space chunks before `PLTE`, the rest before `IDAT`.

use gamut_deflate::{DeflateEncoder, Level};

use crate::chunk;

/// The rendering intent for an `sRGB` chunk (PNG spec §11.3.3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SrgbIntent {
    /// Perceptual (intent code 0).
    Perceptual,
    /// Relative colorimetric (intent code 1).
    RelativeColorimetric,
    /// Saturation (intent code 2).
    Saturation,
    /// Absolute colorimetric (intent code 3).
    AbsoluteColorimetric,
}

impl SrgbIntent {
    fn code(self) -> u8 {
        match self {
            SrgbIntent::Perceptual => 0,
            SrgbIntent::RelativeColorimetric => 1,
            SrgbIntent::Saturation => 2,
            SrgbIntent::AbsoluteColorimetric => 3,
        }
    }
}

/// The unit for a `pHYs` chunk's pixel dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalUnit {
    /// Unit is unknown; the values give only an aspect ratio (unit code 0).
    Unknown,
    /// Pixels per metre (unit code 1).
    Meter,
}

impl PhysicalUnit {
    fn code(self) -> u8 {
        match self {
            PhysicalUnit::Unknown => 0,
            PhysicalUnit::Meter => 1,
        }
    }
}

/// How a text chunk is encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextKind {
    /// `tEXt`: uncompressed Latin-1.
    Latin1,
    /// `zTXt`: zlib-compressed Latin-1.
    Compressed,
    /// `iTXt`: uncompressed UTF-8.
    International,
}

#[derive(Debug, Clone)]
struct TextEntry {
    keyword: String,
    text: String,
    kind: TextKind,
}

/// Accumulated ancillary metadata to emit alongside the image.
#[derive(Debug, Clone, Default)]
pub(crate) struct Ancillary {
    /// gAMA: image gamma × 100000.
    pub gamma: Option<u32>,
    /// cHRM: white/red/green/blue x,y chromaticities × 100000 (8 values).
    pub chrm: Option<[u32; 8]>,
    /// sRGB: rendering-intent code.
    pub srgb: Option<u8>,
    /// sBIT: significant bits per channel (1–4 values, matching the colour type).
    pub sbit: Option<Vec<u8>>,
    /// bKGD: background colour, pre-serialised to its colour-type-specific bytes.
    pub bkgd: Option<Vec<u8>>,
    /// pHYs: (pixels-per-unit X, Y, unit code).
    pub phys: Option<(u32, u32, u8)>,
    /// tIME: year, month, day, hour, minute, second.
    pub time: Option<[u8; 7]>,
    /// tEXt / zTXt / iTXt entries, emitted in insertion order.
    texts: Vec<TextEntry>,
}

impl Ancillary {
    pub(crate) fn set_srgb(&mut self, intent: SrgbIntent) {
        self.srgb = Some(intent.code());
    }

    pub(crate) fn set_physical(&mut self, x: u32, y: u32, unit: PhysicalUnit) {
        self.phys = Some((x, y, unit.code()));
    }

    pub(crate) fn set_time(&mut self, year: u16, month: u8, day: u8, hour: u8, min: u8, sec: u8) {
        let [yh, yl] = year.to_be_bytes();
        self.time = Some([yh, yl, month, day, hour, min, sec]);
    }

    pub(crate) fn add_text_latin1(&mut self, keyword: &str, text: &str) {
        self.push_text(keyword, text, TextKind::Latin1);
    }

    pub(crate) fn add_text_compressed(&mut self, keyword: &str, text: &str) {
        self.push_text(keyword, text, TextKind::Compressed);
    }

    pub(crate) fn add_text_international(&mut self, keyword: &str, text: &str) {
        self.push_text(keyword, text, TextKind::International);
    }

    fn push_text(&mut self, keyword: &str, text: &str, kind: TextKind) {
        self.texts.push(TextEntry {
            keyword: keyword.to_string(),
            text: text.to_string(),
            kind,
        });
    }

    /// Emits the colour-space chunks that must precede `PLTE` (PNG Table 7).
    pub(crate) fn write_pre_plte(&self, out: &mut Vec<u8>) {
        if let Some(chrm) = self.chrm {
            let mut data = [0u8; 32];
            for (slot, value) in chrm.iter().enumerate() {
                data[slot * 4..slot * 4 + 4].copy_from_slice(&value.to_be_bytes());
            }
            chunk::write_chunk(out, *b"cHRM", &data);
        }
        if let Some(gamma) = self.gamma {
            chunk::write_chunk(out, *b"gAMA", &gamma.to_be_bytes());
        }
        if let Some(sbit) = &self.sbit {
            chunk::write_chunk(out, *b"sBIT", sbit);
        }
        if let Some(intent) = self.srgb {
            chunk::write_chunk(out, *b"sRGB", &[intent]);
        }
    }

    /// Emits the remaining ancillary chunks that precede `IDAT` (after any `PLTE`/`tRNS`).
    pub(crate) fn write_post_plte(&self, out: &mut Vec<u8>) {
        if let Some(bkgd) = &self.bkgd {
            chunk::write_chunk(out, *b"bKGD", bkgd);
        }
        if let Some((x, y, unit)) = self.phys {
            let mut data = [0u8; 9];
            data[0..4].copy_from_slice(&x.to_be_bytes());
            data[4..8].copy_from_slice(&y.to_be_bytes());
            data[8] = unit;
            chunk::write_chunk(out, *b"pHYs", &data);
        }
        if let Some(time) = self.time {
            chunk::write_chunk(out, *b"tIME", &time);
        }
        for entry in &self.texts {
            write_text(out, entry);
        }
    }
}

/// Serialises one text chunk (tEXt / zTXt / iTXt).
fn write_text(out: &mut Vec<u8>, entry: &TextEntry) {
    match entry.kind {
        TextKind::Latin1 => {
            let mut data = entry.keyword.clone().into_bytes();
            data.push(0); // null separator
            data.extend_from_slice(entry.text.as_bytes());
            chunk::write_chunk(out, *b"tEXt", &data);
        }
        TextKind::Compressed => {
            let mut data = entry.keyword.clone().into_bytes();
            data.push(0); // null separator
            data.push(0); // compression method: 0 = zlib/deflate
            DeflateEncoder::new()
                .with_level(Level::Best)
                .zlib_compress(entry.text.as_bytes(), &mut data);
            chunk::write_chunk(out, *b"zTXt", &data);
        }
        TextKind::International => {
            let mut data = entry.keyword.clone().into_bytes();
            data.push(0); // null separator
            data.push(0); // compression flag: 0 = uncompressed
            data.push(0); // compression method
            data.push(0); // empty language tag, then null
            data.push(0); // empty translated keyword, then null
            data.extend_from_slice(entry.text.as_bytes()); // UTF-8 text
            chunk::write_chunk(out, *b"iTXt", &data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find_chunk(png: &[u8], ty: &[u8; 4]) -> Option<Vec<u8>> {
        // Walk the chunk stream (after the 8-byte signature) and return a chunk's data.
        let mut i = 8;
        while i + 12 <= png.len() {
            let len = u32::from_be_bytes([png[i], png[i + 1], png[i + 2], png[i + 3]]) as usize;
            let kind = &png[i + 4..i + 8];
            if kind == ty {
                return Some(png[i + 8..i + 8 + len].to_vec());
            }
            i += 12 + len;
        }
        None
    }

    #[test]
    fn pre_plte_serialisation() {
        let a = Ancillary {
            gamma: Some(45455),
            srgb: Some(SrgbIntent::Perceptual.code()),
            sbit: Some(vec![5, 6, 5]),
            ..Default::default()
        };
        let mut out = vec![0u8; 8]; // fake signature
        a.write_pre_plte(&mut out);
        assert_eq!(
            find_chunk(&out, b"gAMA"),
            Some(45455u32.to_be_bytes().to_vec())
        );
        assert_eq!(find_chunk(&out, b"sRGB"), Some(vec![0]));
        assert_eq!(find_chunk(&out, b"sBIT"), Some(vec![5, 6, 5]));
    }

    #[test]
    fn post_plte_serialisation() {
        let mut a = Ancillary::default();
        a.set_physical(2835, 2835, PhysicalUnit::Meter);
        a.set_time(2026, 6, 13, 1, 2, 3);
        a.add_text_latin1("Title", "hi");
        let mut out = vec![0u8; 8];
        a.write_post_plte(&mut out);
        let phys = find_chunk(&out, b"pHYs").unwrap();
        assert_eq!(&phys[0..4], 2835u32.to_be_bytes());
        assert_eq!(phys[8], 1); // metre
        assert_eq!(
            find_chunk(&out, b"tIME").unwrap(),
            vec![7, 234, 6, 13, 1, 2, 3]
        ); // 2026 = 0x07EA
        assert_eq!(find_chunk(&out, b"tEXt").unwrap(), b"Title\0hi".to_vec());
    }

    #[test]
    fn compressed_text_has_keyword_and_zlib_stream() {
        // The zlib stream's validity is cross-checked end-to-end via libpng in the oracle tests
        // (libpng decompresses zTXt on read); here we just check the framing.
        let mut a = Ancillary::default();
        a.add_text_compressed("Comment", "the quick brown fox");
        let mut out = vec![0u8; 8];
        a.write_post_plte(&mut out);
        let data = find_chunk(&out, b"zTXt").unwrap();
        assert_eq!(&data[..8], b"Comment\0");
        assert_eq!(data[8], 0); // compression method
        assert_eq!(data[9], 0x78); // the zlib CMF byte begins the compressed text
    }
}
