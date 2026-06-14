//! Adler-32 checksum (RFC 1950 §9), the integrity check carried in a zlib stream's trailer.

/// The largest prime smaller than 65536; the two running sums are reduced modulo this.
const MOD_ADLER: u32 = 65521;
/// Largest number of bytes that can be summed before the inner `s2` accumulator can overflow a
/// `u32`; reducing once per chunk lets the hot loop avoid a modulo per byte.
const NMAX: usize = 5552;

/// Computes the Adler-32 checksum of `data`, continuing from `seed`.
///
/// Pass `seed = 1` for a fresh checksum (the zlib initial value). The result is `s2 << 16 | s1`,
/// which is written into the zlib trailer in **big-endian** byte order.
#[must_use]
pub fn adler32(seed: u32, data: &[u8]) -> u32 {
    let mut s1 = seed & 0xffff;
    let mut s2 = (seed >> 16) & 0xffff;
    for chunk in data.chunks(NMAX) {
        for &b in chunk {
            s1 += u32::from(b);
            s2 += s1;
        }
        s1 %= MOD_ADLER;
        s2 %= MOD_ADLER;
    }
    (s2 << 16) | s1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_one() {
        assert_eq!(adler32(1, b""), 1);
    }

    #[test]
    fn known_vector() {
        // Adler-32("Wikipedia") == 0x11E60398.
        assert_eq!(adler32(1, b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn matches_zlib_over_many_inputs() {
        // Cross-check against the reference C implementation across sizes that straddle the NMAX
        // chunk boundary, where a deferred-modulo bug would surface.
        for len in [0usize, 1, 55, 256, 5551, 5552, 5553, 20_000, 70_000] {
            let data: Vec<u8> = (0..len)
                .map(|i| (i.wrapping_mul(31) ^ (i >> 3)) as u8)
                .collect();
            assert_eq!(
                adler32(1, &data),
                zlib_oracle::adler32(1, &data),
                "mismatch at len {len}"
            );
        }
    }

    #[test]
    fn seed_resumes_a_running_checksum() {
        // Feeding the checksum in two halves with the seed threaded through equals one pass.
        let data = b"split this input across two adler32 calls";
        let (a, b) = data.split_at(17);
        let one_pass = adler32(1, data);
        let two_pass = adler32(adler32(1, a), b);
        assert_eq!(one_pass, two_pass);
    }
}
