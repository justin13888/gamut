//! TIFF tag numbers — the 2-byte `Tag` field of an IFD entry.
//!
//! Values are from the TIFF 6.0 specification, §8 (Baseline Field Reference Guide) and the
//! Part 2 extension sections. Only the tags the encoder/decoder act on are named here; unknown
//! tags are still parsed structurally by the reader.

/// `ImageWidth` (256) — the number of columns, i.e. pixels per row.
pub const IMAGE_WIDTH: u16 = 256;
/// `ImageLength` (257) — the number of rows (scanlines).
pub const IMAGE_LENGTH: u16 = 257;
/// `BitsPerSample` (258) — bits per component, one value per sample.
pub const BITS_PER_SAMPLE: u16 = 258;
/// `Compression` (259) — the compression scheme applied to the image data.
pub const COMPRESSION: u16 = 259;
/// `FillOrder` (266) — the logical bit order within a byte (1 = MSB-first, the default).
pub const FILL_ORDER: u16 = 266;
/// `PhotometricInterpretation` (262) — the colour space of the image data.
pub const PHOTOMETRIC_INTERPRETATION: u16 = 262;
/// `StripOffsets` (273) — the byte offset of each strip.
pub const STRIP_OFFSETS: u16 = 273;
/// `SamplesPerPixel` (277) — the number of components per pixel.
pub const SAMPLES_PER_PIXEL: u16 = 277;
/// `RowsPerStrip` (278) — the number of rows in each strip.
pub const ROWS_PER_STRIP: u16 = 278;
/// `StripByteCounts` (279) — the number of (compressed) bytes in each strip.
pub const STRIP_BYTE_COUNTS: u16 = 279;
/// `XResolution` (282) — pixels per resolution unit in the horizontal direction.
pub const X_RESOLUTION: u16 = 282;
/// `YResolution` (283) — pixels per resolution unit in the vertical direction.
pub const Y_RESOLUTION: u16 = 283;
/// `PlanarConfiguration` (284) — chunky (1) or planar (2) component storage.
pub const PLANAR_CONFIGURATION: u16 = 284;
/// `ResolutionUnit` (296) — the unit for `XResolution`/`YResolution`.
pub const RESOLUTION_UNIT: u16 = 296;
/// `Predictor` (317) — the prediction scheme applied before compression.
pub const PREDICTOR: u16 = 317;
/// `ColorMap` (320) — the palette for palette-colour images.
pub const COLOR_MAP: u16 = 320;
/// `TileWidth` (322) — the width of each tile in pixels (a multiple of 16).
pub const TILE_WIDTH: u16 = 322;
/// `TileLength` (323) — the height of each tile in pixels (a multiple of 16).
pub const TILE_LENGTH: u16 = 323;
/// `TileOffsets` (324) — the byte offset of each tile.
pub const TILE_OFFSETS: u16 = 324;
/// `TileByteCounts` (325) — the number of (compressed) bytes in each tile.
pub const TILE_BYTE_COUNTS: u16 = 325;
