// A minimal `extern "C"` shim over the Adobe DNG SDK, mirroring the parse/read flow of the SDK's
// own `dng_validate` tool (source/dng_validate.cpp): open the file, parse the IFDs, build a
// negative, and read its stage-1 (raw) image. If any of that throws, the file is not a valid DNG
// the reference implementation accepts.

#include "dng_auto_ptr.h"
#include "dng_errors.h"
#include "dng_exceptions.h"
#include "dng_file_stream.h"
#include "dng_host.h"
#include "dng_info.h"
#include "dng_negative.h"

// Validates the DNG at `path`, returning `dng_error_none` (0) if the Adobe SDK parses and reads it
// without error, or the SDK error code otherwise.
extern "C" int gdng_validate(const char *path) {
  try {
    dng_file_stream stream(path);

    dng_host host;

    dng_info info;
    info.Parse(host, stream);
    info.PostParse(host);

    if (!info.IsValidDNG()) {
      return dng_error_bad_format;
    }

    AutoPtr<dng_negative> negative(host.Make_dng_negative());

    negative->Parse(host, stream, info);
    negative->PostParse(host, stream, info);

    negative->ReadStage1Image(host, stream, info);

    negative->ValidateRawImageDigest(host);
  } catch (const dng_exception &except) {
    return except.ErrorCode();
  } catch (...) {
    return dng_error_unknown;
  }

  return dng_error_none;
}
