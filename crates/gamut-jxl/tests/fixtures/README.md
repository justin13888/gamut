# Test fixtures

## `tiny_baseline.jpg`

A 369-byte, 16×16 baseline (non-progressive) JPEG used by the `jbrd.rs` JPEG-recompression
tests, where the flagship assertion is that libjxl reconstructs these exact bytes from the
gamut-encoded JPEG XL stream.

Provenance (fully deterministic):

1. A 16×16 P6 PPM whose pixel at `(x, y)` is
   `R = (x*16 + 8) & 0xFF`, `G = (y*16 + 8) & 0xFF`, `B = ((x ^ y) * 16) & 0xFF`.
2. `cjpeg -quality 90 -baseline -optimize -outfile tiny_baseline.jpg gradient.ppm`
   with libjpeg-turbo's cjpeg version 3.2.0.

SHA-256: `ea3ec08ab16e25da4a76036bba44dbf112a617b2461b8d7f90a5166844c2b914`

The tests treat the file as an opaque, valid baseline JPEG; regenerating it with a different
cjpeg version would produce different (equally valid) bytes and the tests would still pass —
the byte-exactness being asserted is JPEG→JXL→JPEG reconstruction, not this file's identity.
