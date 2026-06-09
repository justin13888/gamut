# Compression Techniques

At Google, we are constantly looking at ways to make web pages load faster.
One way to do this is by making web images smaller. Images comprise up to
[60%-65% of bytes](https://httparchive.org/interesting.php) on most web pages
and page size is a major factor in total rendering time. Page size is
especially important for mobile devices, where smaller images save both
bandwidth and battery life.

WebP is a new image format developed by Google and supported in Chrome, Opera
and Android that is optimized to enable faster and smaller images on the Web.
WebP images are about
[30% smaller](https://developers.google.com/speed/webp/docs/webp_lossless_alpha_study#results) in size
compared to PNG and JPEG images at equivalent visual quality. In addition, the
WebP image format has feature parity with other formats as well. It supports:

- **Lossy compression:** The lossy compression is based on
  [VP8](https://en.wikipedia.org/wiki/VP8) key frame encoding. VP8 is a video
  compression format created by On2 Technologies as a successor to the VP6
  and VP7 formats.

- **Lossless compression:** The lossless compression format is developed by
  the WebP team.

- **Transparency:** 8-bit alpha channel is useful for graphical images. The
  Alpha channel can be used along with lossy RGB, a feature that's currently
  not available with any other format.

- **Animation:** It supports true-color animated images.

- **Metadata:** It may have EXIF and XMP metadata (used by cameras, for
  example).

- **Color Profile:** It may have an embedded ICC profile.

Due to better compression of images and support for all these features, WebP
can be an excellent replacement for most image formats: PNG, JPEG or GIF. Even
better, did you know that WebP enables new image optimization opportunities,
such as support for lossy images with transparency? Yep! WebP is the Swiss
Army knife of image formats.

So, how is this magic done? Let's roll up our sleeves and take a look under
the hood.

## Lossy WebP

WebP's lossy compression uses the same methodology as VP8 for predicting
(video) frames. VP8 is based on
[block prediction](https://static.googleusercontent.com/external_content/untrusted_dlcp/research.google.com/en/us/pubs/archive/37073.pdf)
and like any block-based codec, VP8 divides the frame into smaller segments
called macroblocks.

Within each macroblock, the encoder can predict redundant motion and color
information based on previously processed blocks. The image frame is "key" in
the sense that it only uses the pixels already decoded in the immediate
spatial neighborhood of each of the macroblocks, and tries to fill in the
unknown part of them. This is called predictive coding (see
[intra-frame coding of the VP8 video](https://blog.webmproject.org/2010/07/inside-webm-technology-vp8-intra-and.html)).

The redundant data can then be subtracted from the block, which results in
more efficient compression. We are only left with a small difference, called
residual, to transmit in a compressed form.

After being subject to a mathematically invertible transform (the famed DCT,
which stands for Discrete Cosine Transform), the residuals typically contain
many zero values, which can be compressed much more effectively. The result is
then quantized and entropy-coded. Amusingly, the quantization step is the only
one where bits are lossy-ly discarded (search for the divide by QPj in the
diagram below). All other steps are invertible and lossless!

The following diagram shows the steps involved in WebP lossy compression. The
differentiating features compared to JPEG are circled in red.

![](https://developers.google.com/static/speed/webp/images/compression-webp_lossy.png)

WebP uses block quantization and distributes bits adaptively across different
image segments: fewer bits for low entropy segments and higher bits for higher
entropy segments. WebP uses
[Arithmetic entropy encoding](https://en.wikipedia.org/wiki/Arithmetic_coding),
achieving better compression compared to the
[Huffman encoding](https://en.wikipedia.org/wiki/Huffman_coding) used in JPEG.

# VP8 Intra-prediction Modes

VP8 intra-prediction modes are used with three types of macroblocks:

- 4x4 luma
- 16x16 luma
- 8x8 chroma

Four common intra-prediction modes are shared by these macroblocks:

- **H_PRED** (horizontal prediction). Fills each column of the block with a
  copy of the left column, L.

- **V_PRED** (vertical prediction). Fills each row of the block with a copy
  of the above row, A.

- **DC_PRED** (DC prediction). Fills the block with a single value using the
  average of the pixels in the row above A and the column to the left of L.

- **TM_PRED** (TrueMotion prediction). A mode that gets its name from a
  compression technique
  [developed by On2 Technologies](https://googlecode.blogspot.com/2011/11/lossless-and-transparency-encoding-in.html).
  In addition to the row A and column L, TM_PRED uses the pixel P above and
  to the left of the block. Horizontal differences between pixels in A
  (starting from P) are propagated using the pixels from L to start each
  row.

The diagram below illustrates the different prediction modes used in WebP
lossy compression.

![](https://developers.google.com/static/speed/webp/images/compression-intra_modes.png)

For 4x4 luma blocks, there are six additional intra modes similar to V_PRED
and H_PRED, but that correspond to predicting pixels in different directions.
More detail on these modes can be found in the
[VP8 Bitstream Guide](https://datatracker.ietf.org/doc/rfc6386/).

### Adaptive Block Quantization

To improve the quality of an image, the image is segmented into areas that
have visibly similar features. For each of these segments, the compression
parameters (quantization steps, filtering strength, etc.) are tuned
independently. This yields efficient compression by redistributing bits to
where they are most useful. VP8 allows a maximum of four segments (a
limitation of the VP8 bitstream).

![](https://developers.google.com/static/speed/webp/images/compression-webp_segment.png)

### Why WebP (lossy) is Better than JPEG

Prediction coding is a main reason WebP wins over JPEG. Block adaptive
quantization makes a big difference, too. Filtering helps at mid/low bitrates.
Boolean arithmetic encoding provides 5%-10% compression gains compared to
Huffman encoding.

## Lossless WebP

The WebP-lossless encoding is based on transforming the image using several
different techniques. Then, entropy coding is performed on the transform
parameters and transformed image data. The transforms applied to the image
include spatial prediction of pixels, color space transform, using locally
emerging palettes, packing multiple pixels into one pixel, and alpha
replacement. For the entropy coding we use a variant of LZ77-Huffman coding,
which uses 2D encoding of distance values and compact sparse values.

### Predictor (Spatial) Transform

Spatial prediction is used to reduce entropy by exploiting the fact that
neighboring pixels are often correlated. In the predictor transform, the
current pixel value is predicted from the pixels that are already decoded (in
scan-line order), and only the residual value (actual - predicted) is encoded.
The prediction mode determines the type of prediction to use. The image is
divided into multiple square regions and all the pixels in one square use the
same prediction mode.

There are 13 different possible prediction modes. Prevalent ones are left,
top, top-left \& top-right pixels. The remaining ones are combinations
(averages) of left, top, top-left and top-right.

### Color (de-correlation) Transform

The goal of the color transform is to decorrelate the R, G and B values of
each pixel. Color transform keeps the green (G) value as it is, transforms red
(R) based on green, and transforms blue (B) based on green and then based on
red.

As is the case for the predictor transform, first the image is divided into
blocks and the same transform mode is used for all the pixels in a block. For
each block there are three types of color transform elements: green_to_red,
green_to_blue, and red_to_blue.

### Subtract Green Transform

The "subtract green transform" subtracts the green values from the red and
blue values of each pixel. When this transform is present, the decoder needs
to add the green value to both red and blue. This is a special case of the
general color decorrelation transform above, frequent enough to warrant a
cutoff.

### Color Indexing (palettes) Transform

If there are not many unique pixel values, it may be more efficient to create
a color index array and replace the pixel values by the array's indices. The
color indexing transform achieves this. The color indexing transform checks
for the number of unique ARGB values in the image. If that number is below a
threshold (256), it creates an array of those ARGB values, which is then used
to replace the pixel values with the corresponding index.

### Color Cache Coding

Lossless WebP compression uses already-seen image fragments in order to
reconstruct new pixels. It can also use a local palette if no interesting
match is found. This palette is continuously updated to reuse recent colors.
In the picture below, you can see the local color cache in action being
updated progressively with the 32 recently-used colors as the scan goes
downward.

![](https://developers.google.com/static/speed/webp/images/compression-beach2.jpg)

### LZ77 Backward Reference

Backward references are tuples of length and distance code. Length indicates
how many pixels in scan-line order are to be copied. Distance code is a number
indicating the position of a previously seen pixel, from which the pixels are
to be copied. The length and distance values are stored using LZ77 prefix
coding.

LZ77 prefix coding divides large integer values into two parts: the prefix
code and the extra bits. The prefix code is stored using an entropy code,
while the extra bits are stored as they are (without an entropy code).

The diagram below illustrates the LZ77 (2D variant) with word-matching
(instead of pixels).

![](https://developers.google.com/static/speed/webp/images/compression-lz77.png)

## Lossy WebP with Alpha

In addition to lossy WebP (RGB colors) and lossless WebP (lossless RGB with
alpha), there's another WebP mode that allows lossy encoding for RGB channels
and lossless encoding for the alpha channel. Such a possibility (lossy RGB and
lossless alpha) is not available today with any of the existing image formats.
Today, webmasters who need transparency must encode images losslessly in PNG,
leading to a significant size bloat. WebP alpha encodes images with low bits-
per-pixel and provides an effective way to reduce the size of such images.
Lossless compression of the alpha channel adds just
[22% bytes](https://developers.google.com/speed/webp/docs/webp_lossless_alpha_study#results)
over lossy (quality 90) WebP encoding.

Overall, replacing transparent PNG with lossy+alpha WebP gives
[60-70%](https://developers.google.com/speed/webp/lossless_alpha_study/compression_ratio_20120709.png)
size saving on average. This has been confirmed as a great attracting feature
for icon-rich mobile sites
([everything.me](https://github.com/EverythingMe/webp-test#readme), for
example).