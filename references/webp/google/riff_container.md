# WebP Container Specification

## Introduction

WebP is an image format that uses either (i) the VP8 key frame encoding to
compress image data in a lossy way or (ii) the WebP lossless encoding. These
encoding schemes should make it more efficient than older formats, such as JPEG,
GIF, and PNG. It is optimized for fast image transfer over the network (for
example, for websites). The WebP format has feature parity (color profile,
metadata, animation, etc.) with other formats as well. This document describes
the structure of a WebP file.

The WebP container (that is, the RIFF container for WebP) allows feature support
over and above the basic use case of WebP (that is, a file containing a single
image encoded as a VP8 key frame). The WebP container provides additional
support for the following:

- Lossless Compression: An image can be losslessly compressed, using the
  WebP Lossless Format.

- Metadata: An image may have metadata stored in Exchangeable Image File
  Format (Exif) or Extensible Metadata Platform (XMP) format.

- Transparency: An image may have transparency, that is, an alpha channel.

- Color Profile: An image may have an embedded ICC profile as described
  by the [International Color Consortium](https://www.color.org/icc_specs2.xalter).

- Animation: An image may have multiple frames with pauses between them,
  making it an animation.

## Naming

It is RECOMMENDED to use the following types when referring to the WebP
container:

|---|---|
| Container Format Name | WebP |
| Filename Extension | .webp |
| MIME-type | image/webp |
| Uniform Type Identifier | org.webmproject.webp |

## Terminology \& Basics

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT", "SHOULD",
"SHOULD NOT", "RECOMMENDED", "NOT RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in BCP 14 [RFC 2119](https://datatracker.ietf.org/doc/html/rfc2119) [RFC 8174](https://datatracker.ietf.org/doc/html/rfc8174)
when, and only when, they appear in all capitals, as shown here.

A WebP file contains either a still image (that is, an encoded matrix of pixels)
or an [animation](https://developers.google.com/speed/webp/docs/riff_container#animation). Optionally, it can also contain transparency
information, a color profile and metadata. We refer to the matrix of pixels as
the *canvas* of the image.

Bit numbering in chunk diagrams starts at `0` for the most significant bit
('MSB 0'), as described in [RFC 1166](https://datatracker.ietf.org/doc/html/rfc1166).

Below are additional terms used throughout this document:

*Reader/Writer*
:   Code that reads WebP files is referred to as a *reader* , while code that
    writes them is referred to as a *writer*.

*uint16*
:   A 16-bit, little-endian, unsigned integer.

*uint24*
:   A 24-bit, little-endian, unsigned integer.

*uint32*
:   A 32-bit, little-endian, unsigned integer.

*FourCC*
:   A four-character code (FourCC) is a *uint32* created by concatenating four
    ASCII characters in little-endian order. This means 'aaaa' (0x61616161) and
    'AAAA' (0x41414141) are treated as different *FourCCs*.

*1-based*
:   An unsigned integer field storing values offset by `-1`, for example, such a
    field would store value *25* as *24*.

*ChunkHeader('ABCD')*
:   Used to describe the *FourCC* and *Chunk Size* header of individual chunks,
    where 'ABCD' is the FourCC for the chunk. This element's size is 8 bytes.

## RIFF File Format

The WebP file format is based on the RIFF (Resource Interchange File Format)
document format.

The basic element of a RIFF file is a *chunk*. It consists of:

     0                   1                   2                   3
     0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                         Chunk FourCC                          |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                          Chunk Size                           |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    :                         Chunk Payload                         :
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

Chunk FourCC: 32 bits
:   ASCII four-character code used for chunk identification.

Chunk Size: 32 bits (*uint32*)
:   The size of the chunk in bytes, not including this field, the chunk
    identifier, or padding.

Chunk Payload: *Chunk Size* bytes
:   The data payload. If *Chunk Size* is odd, a single padding byte -- which MUST
    be `0` to conform with RIFF -- is added.

**Note**: RIFF has a convention that all-uppercase chunk FourCCs are standard
chunks that apply to any RIFF file format, while FourCCs specific to a file
format are all lowercase. WebP does not follow this convention.

## WebP File Header

     0                   1                   2                   3
     0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |      'R'      |      'I'      |      'F'      |      'F'      |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                           File Size                           |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |      'W'      |      'E'      |      'B'      |      'P'      |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

'RIFF': 32 bits
:   The ASCII characters 'R', 'I', 'F', 'F'.

File Size: 32 bits (*uint32*)
:   The size of the file in bytes, starting at offset 8. The maximum value of
    this field is 2\^32 minus 10 bytes and thus the size of the whole file is at
    most 4 GiB minus 2 bytes.

'WEBP': 32 bits
:   The ASCII characters 'W', 'E', 'B', 'P'.

A WebP file MUST begin with a RIFF header with the FourCC 'WEBP'. The file size
in the header is the total size of the chunks that follow plus `4` bytes for
the 'WEBP' FourCC. The file SHOULD NOT contain any data after the data
specified by *File Size*. Readers MAY parse such files, ignoring the trailing
data. As the size of any chunk is even, the size given by the RIFF header is
also even. The contents of individual chunks are described in the following
sections.

## Simple File Format (Lossy)

This layout SHOULD be used if the image requires *lossy* encoding and does not
require transparency or other advanced features provided by the extended format.
Files with this layout are smaller and supported by older software.

Simple WebP (lossy) file format:

     0                   1                   2                   3
     0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                                                               |
    |                    WebP file header (12 bytes)                |
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    :                        'VP8 ' Chunk                           :
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

'VP8 ' Chunk:

     0                   1                   2                   3
     0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                      ChunkHeader('VP8 ')                      |
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    :                           VP8 data                            :
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

VP8 data: *Chunk Size* bytes
:   VP8 bitstream data.

Note that the fourth character in the 'VP8 ' FourCC is an ASCII space (0x20).

The VP8 bitstream format specification is described in [VP8 Data Format and
Decoding Guide](https://datatracker.ietf.org/doc/html/rfc6386). Note that the VP8 frame header contains the VP8 frame
width and height. That is assumed to be the width and height of the canvas.

The VP8 specification describes how to decode the image into Y'CbCr format. To
convert to RGB, [Recommendation BT.601](https://www.itu.int/rec/R-REC-BT.601) SHOULD be used. Applications MAY
use another conversion method, but visual results may differ among decoders.

## Simple File Format (Lossless)

**Note**: Older readers may not support files using the lossless format.

This layout SHOULD be used if the image requires *lossless* encoding (with an
optional transparency channel) and does not require advanced features provided
by the extended format.

Simple WebP (lossless) file format:

     0                   1                   2                   3
     0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                                                               |
    |                    WebP file header (12 bytes)                |
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    :                         'VP8L' Chunk                          :
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

'VP8L' Chunk:

     0                   1                   2                   3
     0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                      ChunkHeader('VP8L')                      |
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    :                           VP8L data                           :
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

VP8L data: *Chunk Size* bytes
:   VP8L bitstream data.

The current specification of the VP8L bitstream can be found at
[WebP Lossless Bitstream Format](https://developers.google.com/speed/webp/docs/webp_lossless_bitstream_specification). Note that the VP8L header
contains the VP8L image width and height. That is assumed to be the width
and height of the canvas.

## Extended File Format

**Note**: Older readers may not support files using the extended format.

An extended format file consists of:

- A 'VP8X' Chunk with information about features used in the file.

- An optional 'ICCP' Chunk with a color profile.

- An optional 'ANIM' Chunk with animation control data.

- Image data.

- An optional 'EXIF' Chunk with Exif metadata.

- An optional 'XMP ' Chunk with XMP metadata.

- An optional list of [unknown chunks](https://developers.google.com/speed/webp/docs/riff_container#unknown_chunks).

For a *still image* , the *image data* consists of a single frame, which is made
up of:

- An optional [alpha subchunk](https://developers.google.com/speed/webp/docs/riff_container#alpha).

- A [bitstream subchunk](https://developers.google.com/speed/webp/docs/riff_container#bitstream_vp8vp8l).

For an *animated image* , the *image data* consists of multiple frames. More
details about frames can be found in the [Animation](https://developers.google.com/speed/webp/docs/riff_container#animation) section.

All chunks necessary for reconstruction and color correction, that is, 'VP8X',
'ICCP', 'ANIM', 'ANMF', 'ALPH', 'VP8 ', and 'VP8L', MUST appear in the order
described earlier. Readers SHOULD fail when chunks necessary for reconstruction
and color correction are out of order.

[Metadata](https://developers.google.com/speed/webp/docs/riff_container#metadata) and [unknown chunks](https://developers.google.com/speed/webp/docs/riff_container#unknown_chunks) MAY appear out of
order.

**Rationale:** The chunks necessary for reconstruction should appear first in
the file to allow a reader to begin decoding an image before receiving all of
the data. An application may benefit from varying the order of metadata and
custom chunks to suit the implementation.

Extended WebP file header:

     0                   1                   2                   3
     0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                                                               |
    |                   WebP file header (12 bytes)                 |
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                      ChunkHeader('VP8X')                      |
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |Rsv|I|L|E|X|A|R|                   Reserved                    |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |          Canvas Width Minus One               |             ...
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    ...  Canvas Height Minus One    |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

Reserved (Rsv): 2 bits
:   MUST be `0`. Readers MUST ignore this field.

ICC profile (I): 1 bit
:   Set if the file contains an 'ICCP' Chunk.

Alpha (L): 1 bit
:   Set if any of the frames of the image contain transparency information
    ("alpha").

Exif metadata (E): 1 bit
:   Set if the file contains Exif metadata.

XMP metadata (X): 1 bit
:   Set if the file contains XMP metadata.

Animation (A): 1 bit
:   Set if this is an animated image. Data in 'ANIM' and 'ANMF' Chunks should be
    used to control the animation.

Reserved (R): 1 bit
:   MUST be `0`. Readers MUST ignore this field.

Reserved: 24 bits
:   MUST be `0`. Readers MUST ignore this field.

Canvas Width Minus One: 24 bits
:   *1-based* width of the canvas in pixels.
    The actual canvas width is `1 + Canvas Width Minus One`.

Canvas Height Minus One: 24 bits
:   *1-based* height of the canvas in pixels.
    The actual canvas height is `1 + Canvas Height Minus One`.

The product of *Canvas Width* and *Canvas Height* MUST be at most `2^32 - 1`.

Future specifications may add more fields. Unknown fields MUST be ignored.

### Animation

An animation is controlled by 'ANIM' and 'ANMF' Chunks.

'ANIM' Chunk:

For an animated image, this chunk contains the *global parameters* of the
animation.

     0                   1                   2                   3
     0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                      ChunkHeader('ANIM')                      |
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                       Background Color                        |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |          Loop Count           |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

Background Color: 32 bits (*uint32*)
:   The default background color of the canvas in \[Blue, Green, Red, Alpha\]
    byte order. This color MAY be used to fill the unused space on the canvas
    around the frames, as well as the transparent pixels of the first frame.
    The background color is also used when the Disposal method is `1`.

**Notes**:

- The background color MAY contain a non-opaque alpha value, even if the
  *Alpha* flag in the ['VP8X' Chunk](https://developers.google.com/speed/webp/docs/riff_container#extended_header) is unset.

- Viewer applications SHOULD treat the background color value as a hint and
  are not required to use it.

- The canvas is cleared at the start of each loop. The background color MAY be
  used to achieve this.

Loop Count: 16 bits (*uint16*)
:   The number of times to loop the animation. If it is `0`, this means
    infinitely.

This chunk MUST appear if the *Animation* flag in the 'VP8X' Chunk is set.
If the *Animation* flag is not set and this chunk is present, it MUST be
ignored.

'ANMF' Chunk:

For animated images, this chunk contains information about a *single* frame.
If the *Animation flag* is not set, then this chunk SHOULD NOT be present.

     0                   1                   2                   3
     0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                      ChunkHeader('ANMF')                      |
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                        Frame X                |             ...
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    ...          Frame Y            |   Frame Width Minus One     ...
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    ...             |           Frame Height Minus One              |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                 Frame Duration                |  Reserved |B|D|
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    :                         Frame Data                            :
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

Frame X: 24 bits (*uint24*)
:   The X coordinate of the upper left corner of the frame is `Frame X * 2`.

Frame Y: 24 bits (*uint24*)
:   The Y coordinate of the upper left corner of the frame is `Frame Y * 2`.

Frame Width Minus One: 24 bits (*uint24*)
:   The *1-based* width of the frame.
    The frame width is `1 + Frame Width Minus One`.

Frame Height Minus One: 24 bits (*uint24*)
:   The *1-based* height of the frame.
    The frame height is `1 + Frame Height Minus One`.

Frame Duration: 24 bits (*uint24*)
:   The time to wait before displaying the next frame, in 1-millisecond units.
    Note that the interpretation of the Frame Duration of 0 (and often \<= 10) is
    defined by the implementation. Many tools and browsers assign a minimum
    duration similar to GIF.

Reserved: 6 bits
:   MUST be `0`. Readers MUST ignore this field.

Blending method (B): 1 bit

:   Indicates how transparent pixels of *the current frame* are to be blended
    with corresponding pixels of the previous canvas:

    - `0`: Use alpha-blending. After disposing of the previous frame, render the
      current frame on the canvas using alpha-blending (see below). If the
      current frame does not have an alpha channel, assume the alpha value is
      255, effectively replacing the rectangle.

    - `1`: Do not blend. After disposing of the previous frame, render the
      current frame on the canvas by overwriting the rectangle covered by the
      current frame.

Disposal method (D): 1 bit

:   Indicates how *the current frame* is to be treated after it has been
    displayed (before rendering the next frame) on the canvas:

    - `0`: Do not dispose. Leave the canvas as is.

    - `1`: Dispose to the background color. Fill the *rectangle* on the canvas
      covered by the *current frame* with the background color specified in the
      ['ANIM' Chunk](https://developers.google.com/speed/webp/docs/riff_container#anim_chunk).

**Notes**:

- The frame disposal only applies to the *frame rectangle* , that is, the
  rectangle defined by *Frame X* , *Frame Y* , *frame width* , and *frame
  height*. It may or may not cover the whole canvas.

- Alpha-blending:

  Given that each of the R, G, B, and A channels is 8 bits, and the RGB
  channels are *not premultiplied* by alpha, the formula for blending
  'dst' onto 'src' is:

      blend.A = src.A + dst.A * (1 - src.A / 255)
      if blend.A = 0 then
        blend.RGB = 0
      else
        blend.RGB =
            (src.RGB * src.A +
             dst.RGB * dst.A * (1 - src.A / 255)) / blend.A

- Alpha-blending SHOULD be done in linear color space, by taking into account
  the [color profile](https://developers.google.com/speed/webp/docs/riff_container#color_profile) of the image. If the color profile is
  not present, standard RGB (sRGB) is to be assumed. (Note that sRGB also
  needs to be linearized due to a gamma of \~2.2.)

Frame Data: *Chunk Size* bytes - `16`

:   Consists of:

    - An optional [alpha subchunk](https://developers.google.com/speed/webp/docs/riff_container#alpha) for the frame.

    - A [bitstream subchunk](https://developers.google.com/speed/webp/docs/riff_container#bitstream_vp8vp8l) for the frame.

    - An optional list of [unknown chunks](https://developers.google.com/speed/webp/docs/riff_container#unknown_chunks).

**Note** : The 'ANMF' payload, *Frame Data* , consists of individual
*padded* chunks, as described by the [RIFF file format](https://developers.google.com/speed/webp/docs/riff_container#riff_file_format).

### Alpha

     0                   1                   2                   3
     0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                      ChunkHeader('ALPH')                      |
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |Rsv| P | F | C |     Alpha Bitstream...                        |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

Reserved (Rsv): 2 bits
:   MUST be `0`. Readers MUST ignore this field.

Preprocessing (P): 2 bits

:   These *informative* bits are used to signal the preprocessing that has
    been performed during compression. The decoder can use this information to
    for example, dither the values or smooth the gradients prior to display.

    - `0`: No preprocessing.
    - `1`: Level reduction.

Decoders are not required to use this information in any specified way.

Filtering method (F): 2 bits

:   The filtering methods used are described as follows:

    - `0`: None.
    - `1`: Horizontal filter.
    - `2`: Vertical filter.
    - `3`: Gradient filter.

For each pixel, filtering is performed using the following calculations.
Assume the alpha values surrounding the current `X` position are labeled as:

     C | B |
    ---+---+
     A | X |

We seek to compute the alpha value at position `X`. First, a prediction is
made depending on the filtering method:

- Method `0`: predictor = 0
- Method `1`: predictor = A
- Method `2`: predictor = B
- Method `3`: predictor = clip(A + B - C)

where `clip(v)` is equal to:

- 0 if v \< 0,
- 255 if v \> 255, or
- v otherwise

The final value is derived by adding the decompressed value `X` to the
predictor and using modulo-256 arithmetic to wrap the \[256..511\] range
into the \[0..255\] one:

`alpha = (predictor + X) % 256`

There are special cases for the left-most and top-most pixel positions. For
example, the top-left value at location (0, 0) uses 0 as the predictor value.
Otherwise:

- For horizontal or gradient filtering methods, the left-most pixels at location (0, y) are predicted using the location (0, y-1) just above.
- For vertical or gradient filtering methods, the top-most pixels at location (x, 0) are predicted using the location (x-1, 0) on the left.

Compression method (C): 2 bits

:   The compression method used:

    - `0`: No compression.
    - `1`: Compressed using the WebP lossless format.

Alpha bitstream: *Chunk Size* bytes - `1`

:   Encoded alpha bitstream.

This optional chunk contains encoded alpha data for this frame. A frame
containing a 'VP8L' Chunk SHOULD NOT contain this chunk.

**Rationale**: The transparency information is already part of the 'VP8L'
Chunk.

The alpha channel data is stored as uncompressed raw data (when the
compression method is '0') or compressed using the lossless format
(when the compression method is '1').

- Raw data: This consists of a byte sequence of length = width \* height,
  containing all the 8-bit transparency values in scan order.

- Lossless format compression: The byte sequence is a compressed
  image-stream (as described in ["WebP Lossless Bitstream Format"](https://developers.google.com/speed/webp/docs/webp_lossless_bitstream_specification)) of implicit dimensions width x height. That is, this
  image-stream does NOT contain any headers describing the image dimensions.

  **Rationale**: The dimensions are already known from other sources,
  so storing them again would be redundant and prone to error.

  Once the image-stream is decoded into Alpha, Red, Green, Blue (ARGB) color
  values, following the process described in the lossless format
  specification, the transparency information must be extracted from the
  *green* channel of the ARGB quadruplet.

  **Rationale**: The green channel is allowed extra transformation
  steps in the specification -- unlike the other channels -- that can
  improve compression.

### Bitstream (VP8/VP8L)

This chunk contains compressed bitstream data for a single frame.

A bitstream chunk may be either (i) a 'VP8 ' Chunk, using 'VP8 ' (note the
significant fourth-character space) as its FourCC, *or* (ii) a 'VP8L' Chunk,
using 'VP8L' as its FourCC.

The formats of 'VP8 ' and 'VP8L' Chunks are as described in sections
[Simple File Format (Lossy)](https://developers.google.com/speed/webp/docs/riff_container#simple_file_format_lossy)
and [Simple File Format (Lossless)](https://developers.google.com/speed/webp/docs/riff_container#simple_file_format_lossless) respectively.

### Color Profile

     0                   1                   2                   3
     0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                      ChunkHeader('ICCP')                      |
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    :                       Color Profile                           :
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

Color Profile: *Chunk Size* bytes
:   ICC profile.

This chunk MUST appear before the image data.

There SHOULD be at most one such chunk. If there are more such chunks, readers
MAY ignore all except the first one.
See the [ICC Specification](https://www.color.org/icc_specs2.xalter) for details.

If this chunk is not present, sRGB SHOULD be assumed.

### Metadata

Metadata can be stored in 'EXIF' or 'XMP ' Chunks.

There SHOULD be at most one chunk of each type ('EXIF' and 'XMP '). If there
are more such chunks, readers MAY ignore all except the first one.

The chunks are defined as follows:

'EXIF' Chunk:

     0                   1                   2                   3
     0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                      ChunkHeader('EXIF')                      |
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    :                        Exif Metadata                          :
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

Exif Metadata: *Chunk Size* bytes
:   Image metadata in Exif format.

'XMP ' Chunk:

     0                   1                   2                   3
     0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1 2 3 4 5 6 7 8 9 0 1
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    |                      ChunkHeader('XMP ')                      |
    |                                                               |
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+
    :                        XMP Metadata                           :
    +-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+-+

XMP Metadata: *Chunk Size* bytes
:   Image metadata in XMP format.

Note that the fourth character in the 'XMP ' FourCC is an ASCII space (0x20).

Additional guidance about handling metadata can be found in the
Metadata Working Group's ["Guidelines for Handling Metadata"](https://web.archive.org/web/20180919181934/http://www.metadataworkinggroup.org/pdf/mwg_guidance.pdf).

### Unknown Chunks

A RIFF chunk (described in the [RIFF File Format](https://developers.google.com/speed/webp/docs/riff_container#riff_file_format) section)
whose FourCC is different from any of the chunks described in this document, is
considered an *unknown chunk*.

**Rationale**: Allowing unknown chunks gives a provision for future extension
of the format and also allows storage of any application-specific data.

A file MAY contain unknown chunks:

- at the end of the file, as described in [Extended WebP file
  header](https://developers.google.com/speed/webp/docs/riff_container#extended_header) section, or
- at the end of 'ANMF' Chunks, as described in the [Animation](https://developers.google.com/speed/webp/docs/riff_container#animation) section.

Readers SHOULD ignore these chunks. Writers SHOULD preserve them in their
original order (unless they specifically intend to modify these chunks).

## Canvas Assembly from Frames

Here we provide an overview of how a reader MUST assemble a canvas in the case
of an animated image.

The process begins with creating a canvas using the dimensions given in the
'VP8X' Chunk, `Canvas Width Minus One + 1` pixels wide by `Canvas Height Minus
One + 1` pixels high. The `Loop Count` field from the 'ANIM' Chunk controls how
many times the animation process is repeated. This is `Loop Count - 1` for
nonzero `Loop Count` values or infinite if the `Loop Count` is zero.

At the beginning of each loop iteration, the canvas is filled using the
background color from the 'ANIM' Chunk or an application-defined color.

'ANMF' Chunks contain individual frames given in display order. Before rendering
each frame, the previous frame's `Disposal method` is applied.

The rendering of the decoded frame begins at the Cartesian coordinates (`2 *
Frame X`, `2 * Frame Y`), using the top-left corner of the canvas as the origin.
`Frame Width Minus One + 1` pixels wide by `Frame Height Minus One + 1` pixels
high are rendered onto the canvas using the `Blending method`.

The canvas is displayed for `Frame Duration` milliseconds. This continues until
all frames given by 'ANMF' Chunks have been displayed. A new loop iteration is
then begun, or the canvas is left in its final state if all iterations have been
completed.

The following pseudocode illustrates the rendering process. The notation
*VP8X.field* means the field in the 'VP8X' Chunk with the same description.

    VP8X.flags.hasAnimation MUST be TRUE
    canvas ← new image of size VP8X.canvasWidth x VP8X.canvasHeight with
             background color ANIM.background_color or
             application-defined color.
    loop_count ← ANIM.loopCount
    dispose_method ← Dispose to background color
    if loop_count == 0:
      loop_count = ∞
    frame_params ← nil
    next chunk in image_data is ANMF MUST be TRUE
    for loop = 0..loop_count - 1
      clear canvas to ANIM.background_color or application-defined color
      until eof or non-ANMF chunk
        frame_params.frameX = Frame X
        frame_params.frameY = Frame Y
        frame_params.frameWidth = Frame Width Minus One + 1
        frame_params.frameHeight = Frame Height Minus One + 1
        frame_params.frameDuration = Frame Duration
        frame_right = frame_params.frameX + frame_params.frameWidth
        frame_bottom = frame_params.frameY + frame_params.frameHeight
        VP8X.canvasWidth >= frame_right MUST be TRUE
        VP8X.canvasHeight >= frame_bottom MUST be TRUE
        for subchunk in 'Frame Data':
          if subchunk.tag == "ALPH":
            alpha subchunks not found in 'Frame Data' earlier MUST be
              TRUE
            frame_params.alpha = alpha_data
          else if subchunk.tag == "VP8 " OR subchunk.tag == "VP8L":
            bitstream subchunks not found in 'Frame Data' earlier MUST
              be TRUE
            frame_params.bitstream = bitstream_data
        apply dispose_method.
        render frame with frame_params.alpha and frame_params.bitstream
          on canvas with top-left corner at (frame_params.frameX,
          frame_params.frameY), using Blending method
          frame_params.blendingMethod.
        canvas contains the decoded image.
        Show the contents of the canvas for
        frame_params.frameDuration * 1 ms.
        dispose_method = frame_params.disposeMethod

## Example File Layouts

A lossy-encoded image with alpha may look as follows:

    RIFF/WEBP
    +- VP8X (descriptions of features used)
    +- ALPH (alpha bitstream)
    +- VP8 (bitstream)

A lossless-encoded image may look as follows:

    RIFF/WEBP
    +- VP8X (descriptions of features used)
    +- VP8L (lossless bitstream)
    +- XYZW (unknown chunk)

A lossless image with an ICC profile and XMP metadata may
look as follows:

    RIFF/WEBP
    +- VP8X (descriptions of features used)
    +- ICCP (color profile)
    +- VP8L (lossless bitstream)
    +- XMP  (metadata)

An animated image with Exif metadata may look as follows:

    RIFF/WEBP
    +- VP8X (descriptions of features used)
    +- ANIM (global animation parameters)
    +- ANMF (frame1 parameters + data)
    +- ANMF (frame2 parameters + data)
    +- ANMF (frame3 parameters + data)
    +- ANMF (frame4 parameters + data)
    +- EXIF (metadata)