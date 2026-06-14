// Link-only stubs for the libjxl C API that the Adobe DNG SDK 1.7.1 references.
//
// In 1.7.1 JPEG XL is wired into the SDK unconditionally (the `qDNGSupportJXL` switch was removed),
// so `dng_jxl.cpp` and the writer/reader translation units reference these symbols even when no
// JPEG XL image is ever touched. gamut-dng's baseline oracle validates only uncompressed /
// lossless-JPEG / Deflate DNGs, so none of these are ever *called* — they exist solely so the
// archive links without a real libjxl.
//
// C linkage means the linker resolves these by name alone; the trivial signatures below need not
// match the real libjxl prototypes (the functions are never invoked). If the SDK is updated and
// references a new symbol, the linker will name it and it can be added here.

extern "C" {

// --- Decoder -------------------------------------------------------------------------------------
void *JxlDecoderCreate(const void *m) { (void)m; return 0; }
void JxlDecoderDestroy(void *d) { (void)d; }
void JxlDecoderReset(void *d) { (void)d; }
int JxlDecoderSubscribeEvents(void *d, int e) { (void)d; (void)e; return 1; }
int JxlDecoderSetParallelRunner(void *d, void *r, void *p) { (void)d; (void)r; (void)p; return 1; }
int JxlDecoderSetInput(void *d, const unsigned char *b, unsigned long n) { (void)d; (void)b; (void)n; return 1; }
void JxlDecoderCloseInput(void *d) { (void)d; }
unsigned long JxlDecoderReleaseInput(void *d) { (void)d; return 0; }
int JxlDecoderProcessInput(void *d) { (void)d; return 1; }
int JxlDecoderGetBasicInfo(const void *d, void *i) { (void)d; (void)i; return 1; }
int JxlDecoderGetColorAsEncodedProfile(const void *d, int t, void *e) { (void)d; (void)t; (void)e; return 1; }
int JxlDecoderGetICCProfileSize(const void *d, int t, unsigned long *s) { (void)d; (void)t; (void)s; return 1; }
int JxlDecoderGetColorAsICCProfile(const void *d, int t, unsigned char *b, unsigned long n) { (void)d; (void)t; (void)b; (void)n; return 1; }
int JxlDecoderGetExtraChannelInfo(const void *d, unsigned long i, void *info) { (void)d; (void)i; (void)info; return 1; }
int JxlDecoderGetExtraChannelName(const void *d, unsigned long i, char *n, unsigned long s) { (void)d; (void)i; (void)n; (void)s; return 1; }
int JxlDecoderImageOutBufferSize(const void *d, const void *f, unsigned long *s) { (void)d; (void)f; (void)s; return 1; }
int JxlDecoderSetImageOutBuffer(void *d, const void *f, void *b, unsigned long s) { (void)d; (void)f; (void)b; (void)s; return 1; }
int JxlDecoderSetImageOutCallback(void *d, const void *f, void *cb, void *o) { (void)d; (void)f; (void)cb; (void)o; return 1; }
int JxlDecoderGetFrameHeader(const void *d, void *h) { (void)d; (void)h; return 1; }
int JxlDecoderGetFrameName(const void *d, char *n, unsigned long s) { (void)d; (void)n; (void)s; return 1; }
int JxlDecoderSetKeepOrientation(void *d, int k) { (void)d; (void)k; return 1; }
int JxlDecoderSetDecompressBoxes(void *d, int b) { (void)d; (void)b; return 1; }
int JxlDecoderSetBoxBuffer(void *d, unsigned char *b, unsigned long n) { (void)d; (void)b; (void)n; return 1; }
unsigned long JxlDecoderReleaseBoxBuffer(void *d) { (void)d; return 0; }
int JxlDecoderGetBoxType(void *d, char *t, int r) { (void)d; (void)t; (void)r; return 1; }
int JxlDecoderGetBoxSizeRaw(const void *d, unsigned long *s) { (void)d; (void)s; return 1; }

// --- Encoder -------------------------------------------------------------------------------------
void *JxlEncoderCreate(const void *m) { (void)m; return 0; }
void JxlEncoderDestroy(void *e) { (void)e; }
void JxlEncoderReset(void *e) { (void)e; }
int JxlEncoderSetParallelRunner(void *e, void *r, void *p) { (void)e; (void)r; (void)p; return 1; }
void *JxlEncoderFrameSettingsCreate(void *e, const void *s) { (void)e; (void)s; return 0; }
int JxlEncoderFrameSettingsSetOption(void *s, int o, long v) { (void)s; (void)o; (void)v; return 1; }
int JxlEncoderSetFrameDistance(void *s, float d) { (void)s; (void)d; return 1; }
int JxlEncoderSetFrameLossless(void *s, int l) { (void)s; (void)l; return 1; }
int JxlEncoderSetFrameName(void *s, const char *n) { (void)s; (void)n; return 1; }
void JxlEncoderInitBasicInfo(void *i) { (void)i; }
int JxlEncoderSetBasicInfo(void *e, const void *i) { (void)e; (void)i; return 1; }
int JxlEncoderSetColorEncoding(void *e, const void *c) { (void)e; (void)c; return 1; }
int JxlEncoderSetICCProfile(void *e, const unsigned char *b, unsigned long n) { (void)e; (void)b; (void)n; return 1; }
int JxlEncoderSetCodestreamLevel(void *e, int l) { (void)e; (void)l; return 1; }
int JxlEncoderUseContainer(void *e, int u) { (void)e; (void)u; return 1; }
int JxlEncoderUseBoxes(void *e) { (void)e; return 1; }
int JxlEncoderAddBox(void *e, const char *t, const unsigned char *b, unsigned long n, int c) { (void)e; (void)t; (void)b; (void)n; (void)c; return 1; }
void JxlEncoderCloseBoxes(void *e) { (void)e; }
void JxlEncoderCloseInput(void *e) { (void)e; }
int JxlEncoderAddImageFrame(void *s, const void *f, const void *b, unsigned long n) { (void)s; (void)f; (void)b; (void)n; return 1; }
int JxlEncoderAddChunkedFrame(void *s, int l, void *src) { (void)s; (void)l; (void)src; return 1; }
int JxlEncoderSetOutputProcessor(void *e, void *p) { (void)e; (void)p; return 1; }
int JxlEncoderProcessOutput(void *e, unsigned char **n, unsigned long *a) { (void)e; (void)n; (void)a; return 1; }

// --- Parallel runners ----------------------------------------------------------------------------
void *JxlResizableParallelRunnerCreate(const void *m) { (void)m; return 0; }
void JxlResizableParallelRunnerDestroy(void *r) { (void)r; }
void JxlResizableParallelRunnerSetThreads(void *r, unsigned long n) { (void)r; (void)n; }
int JxlResizableParallelRunner(void *r, void *o, void *init, void *run, unsigned int n) { (void)r; (void)o; (void)init; (void)run; (void)n; return 0; }
unsigned long JxlResizableParallelRunnerSuggestThreads(unsigned long w, unsigned long h) { (void)w; (void)h; return 1; }
void *JxlThreadParallelRunnerCreate(const void *m, unsigned long n) { (void)m; (void)n; return 0; }
void JxlThreadParallelRunnerDestroy(void *r) { (void)r; }
int JxlThreadParallelRunner(void *r, void *o, void *init, void *run, unsigned int n) { (void)r; (void)o; (void)init; (void)run; (void)n; return 0; }

} // extern "C"
