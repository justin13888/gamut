/* bindgen entry point for the libaom oracle. Pulls in libaom's public decode + encode
 * API and declares the macro-unwrapping shim (see shim.c). All headers live under
 * `aom/` at the vendored source root, reached via the `-I<src>` clang arg in build.rs. */
#include <aom/aom_decoder.h>
#include <aom/aomdx.h>
#include <aom/aom_encoder.h>
#include <aom/aomcx.h>
#include <aom/aom_image.h>

/* `aom_codec_dec_init` / `aom_codec_enc_init` are function-like macros that append the
 * ABI-version constant (`AOM_{DECODER,ENCODER}_ABI_VERSION`). Those constants are nested
 * `#define` expressions bindgen cannot const-fold, so it never emits them and the macros
 * are unusable from Rust. These non-macro wrappers give the Rust side a stable, linkable
 * entry point with the correct ABI version baked in at C-compile time. */
aom_codec_err_t aomshim_dec_init(aom_codec_ctx_t *ctx, aom_codec_iface_t *iface);
aom_codec_err_t aomshim_enc_init(aom_codec_ctx_t *ctx, aom_codec_iface_t *iface,
                                 const aom_codec_enc_cfg_t *cfg);
