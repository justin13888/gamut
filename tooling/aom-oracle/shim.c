/* Real functions wrapping libaom's `aom_codec_{dec,enc}_init` macros, so the ABI-version
 * constant they inject is resolved here at C-compile time (bindgen cannot emit it — it is a
 * nested `#define` expression). See wrapper.h for the rationale. */
#include "wrapper.h"

aom_codec_err_t aomshim_dec_init(aom_codec_ctx_t *ctx, aom_codec_iface_t *iface) {
  return aom_codec_dec_init(ctx, iface, NULL, 0);
}

aom_codec_err_t aomshim_enc_init(aom_codec_ctx_t *ctx, aom_codec_iface_t *iface,
                                 const aom_codec_enc_cfg_t *cfg) {
  return aom_codec_enc_init(ctx, iface, cfg, 0);
}
