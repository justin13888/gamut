/* bindgen entry point for the HEIF decode/encode oracle.
 *
 * Pulls in the whole vendored libheif public API (the container + HEVC image items) and the
 * libde265 public decode API. The latter is exposed so gamut-heic's tests can feed raw HEVC NAL
 * units straight to the reference HEVC decoder — bypassing libheif — behind its pluggable-decoder
 * trait. */
#include <libheif/heif.h>
/* heif.h does not pull in the items API (item id/type enumeration), so include it explicitly for
 * the container-structure introspection. */
#include <libheif/heif_items.h>
#include <libde265/de265.h>
