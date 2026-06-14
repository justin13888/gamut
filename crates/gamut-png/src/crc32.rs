//! CRC-32 for PNG chunk integrity (PNG spec §5.3, the ISO-3309 / ITU-T V.42 polynomial).
//!
//! This is the reflected CRC-32 with polynomial `0xEDB88320`, initial value all-ones, and a final
//! ones-complement, computed over a chunk's **type and data** (not its length). zlib uses Adler-32,
//! never this — so CRC-32 lives in the PNG crate, not in `gamut-deflate`.

/// Precomputed byte-wise CRC table (built at compile time).
const TABLE: [u32; 256] = build_table();

const fn build_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut n = 0usize;
    while n < 256 {
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        table[n] = c;
        n += 1;
    }
    table
}

/// An incremental CRC-32 accumulator.
pub(crate) struct Crc32 {
    value: u32,
}

impl Crc32 {
    /// Starts a fresh CRC (register initialised to all ones).
    pub(crate) fn new() -> Self {
        Self { value: 0xFFFF_FFFF }
    }

    /// Folds `data` into the running CRC.
    pub(crate) fn update(&mut self, data: &[u8]) {
        let mut crc = self.value;
        for &b in data {
            crc = TABLE[((crc ^ u32::from(b)) & 0xff) as usize] ^ (crc >> 8);
        }
        self.value = crc;
    }

    /// Finalises the CRC (ones-complement of the register).
    pub(crate) fn finish(self) -> u32 {
        self.value ^ 0xFFFF_FFFF
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crc(data: &[u8]) -> u32 {
        let mut c = Crc32::new();
        c.update(data);
        c.finish()
    }

    #[test]
    fn known_chunk_crcs() {
        // The CRC over the bytes "IEND" is the fixed value every PNG's end chunk carries.
        assert_eq!(crc(b"IEND"), 0xAE42_6082);
        // CRC of the empty string is 0.
        assert_eq!(crc(b""), 0);
    }

    #[test]
    fn incremental_matches_one_shot() {
        // Folding in two parts equals folding the whole (chunk writers feed type then data).
        let mut split = Crc32::new();
        split.update(b"IHDR");
        split.update(&[0, 0, 1, 0]);
        let mut whole = Crc32::new();
        whole.update(b"IHDR\x00\x00\x01\x00");
        assert_eq!(split.finish(), whole.finish());
    }
}
