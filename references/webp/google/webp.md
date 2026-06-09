# An image format for the Web

![](https://developers.google.com/static/speed/webp/images/webplogo.png)
WebP is a modern **image format** that provides superior **lossless and
lossy** compression for images on the web. Using WebP, webmasters and web
developers can create smaller, richer images that make the web faster.

WebP lossless images are [26% smaller](https://developers.google.com/speed/webp/docs/webp_lossless_alpha_study#results) in size compared to PNGs. WebP
lossy images are [25-34% smaller](https://developers.google.com/speed/webp/docs/webp_study) than comparable JPEG images at equivalent
[SSIM](https://en.wikipedia.org/wiki/Structural_similarity) quality index.

Lossless WebP **supports transparency** (also known as alpha channel) at a
cost of just [22% additional bytes](https://developers.google.com/speed/webp/docs/webp_lossless_alpha_study#results). For cases when lossy RGB compression
is acceptable, **lossy WebP also supports transparency**, typically providing
3× smaller file sizes compared to PNG.

Lossy, lossless and transparency are all supported in **animated WebP images**,
which can provide reduced sizes compared to GIF and APNG.

- **[More Info for Webmasters](https://developers.google.com/speed/webp/faq#how_can_i_detect_browser_support_for_webp)**

## How WebP Works

Lossy WebP compression uses predictive coding to encode an image, the same
method used by the VP8 video codec to compress keyframes in videos. Predictive
coding uses the values in neighboring blocks of pixels to predict the values
in a block, and then encodes only the difference.

Lossless WebP compression uses already seen image fragments in order to
exactly reconstruct new pixels. It can also use a local palette if no
interesting match is found.

- **[WebP Compression Techniques in Detail](https://developers.google.com/speed/webp/docs/compression)**

A WebP file consists of [VP8](https://datatracker.ietf.org/doc/rfc6386/) or [VP8L](https://developers.google.com/speed/webp/docs/webp_lossless_bitstream_specification) image data, and a container
based on [RIFF](https://developers.google.com/speed/webp/docs/riff_container). The standalone `libwebp` library serves as a reference
implementation for the WebP specification, and is available from
[our git repository](https://www.webmproject.org/code/#libwebp-webp-image-library) or as a [tarball](https://developers.google.com/speed/webp/download).

## WebP Support

WebP is natively supported in Google Chrome, Safari, Firefox, Edge, the Opera
browser, and by [many other](https://developers.google.com/speed/webp/faq#which_web_browsers_natively_support_webp) tools and software libraries. Developers have
also added support to a variety of image editing tools.

WebP includes the lightweight encoding and decoding library [`libwebp`](https://developers.google.com/speed/webp/docs/api)
and the command line tools [`cwebp`](https://developers.google.com/speed/webp/docs/cwebp) and [`dwebp`](https://developers.google.com/speed/webp/docs/dwebp) for converting
images to and from the WebP format, as well as tools for viewing, muxing and
animating WebP images. The full source code is available on the
[download](https://developers.google.com/speed/webp/download) page.

## WebP Converter Download

Convert your favorite collection from PNG and JPEG to WebP by downloading the
precompiled `cwebp` conversion tool for [Linux, Windows or macOS](https://developers.google.com/speed/webp/docs/precompiled).

Tell us your experience on the project's [mailing list](https://groups.google.com/a/webmproject.org/group/webp-discuss).