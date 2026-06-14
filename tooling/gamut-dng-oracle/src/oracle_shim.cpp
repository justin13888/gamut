// A minimal `extern "C"` shim over the Adobe DNG SDK, mirroring the parse/read flow of the SDK's
// own `dng_validate` tool (source/dng_validate.cpp): open the file, parse the IFDs, build a
// negative, and read its stage-1 (raw) image. If any of that throws, the file is not a valid DNG
// the reference implementation accepts.

#include "dng_auto_ptr.h"
#include "dng_errors.h"
#include "dng_exceptions.h"
#include "dng_file_stream.h"
#include "dng_host.h"
#include "dng_image.h"
#include "dng_info.h"
#include "dng_negative.h"
#include "dng_pixel_buffer.h"
#include "dng_rect.h"
#include "dng_tag_types.h"

#include <cstdint>
#include <cstdlib>

namespace {

// Parses `path` into a negative and reads its stage-1 (raw) image. Shared by the entry points.
dng_error_code read_negative(const char *path, dng_host &host, dng_info &info,
                             AutoPtr<dng_negative> &negative) {
  dng_file_stream stream(path);
  info.Parse(host, stream);
  info.PostParse(host);
  if (!info.IsValidDNG()) {
    return dng_error_bad_format;
  }
  negative.Reset(host.Make_dng_negative());
  negative->Parse(host, stream, info);
  negative->PostParse(host, stream, info);
  negative->ReadStage1Image(host, stream, info);
  return dng_error_none;
}

} // namespace

// Validates the DNG at `path`, returning `dng_error_none` (0) if the Adobe SDK parses and reads it
// without error, or the SDK error code otherwise.
extern "C" int gdng_validate(const char *path) {
  try {
    dng_host host;
    dng_info info;
    AutoPtr<dng_negative> negative;
    dng_error_code rc = read_negative(path, host, info, negative);
    if (rc != dng_error_none) {
      return rc;
    }
    negative->ValidateRawImageDigest(host);
  } catch (const dng_exception &except) {
    return except.ErrorCode();
  } catch (...) {
    return dng_error_unknown;
  }

  return dng_error_none;
}

// Reads the DNG at `path` and returns its stage-1 (raw) image samples — the sensor values as
// stored, before linearisation/black-subtraction — as a freshly `malloc`d interleaved `uint16`
// buffer (`width * height * planes`), which the caller must release with `gdng_free`. Returns
// `dng_error_none` on success, or the SDK error code (the raw must be a 16-bit-typed image).
extern "C" int gdng_read_raw(const char *path, uint32_t *out_w, uint32_t *out_h,
                             uint32_t *out_planes, uint16_t **out_data, size_t *out_len) {
  *out_data = nullptr;
  *out_w = 0;
  *out_h = 0;
  *out_planes = 0;
  *out_len = 0;
  try {
    dng_host host;
    dng_info info;
    AutoPtr<dng_negative> negative;
    dng_error_code rc = read_negative(path, host, info, negative);
    if (rc != dng_error_none) {
      return rc;
    }
    const dng_image *image = negative->Stage1Image();
    if (image == nullptr) {
      return dng_error_unknown;
    }
    if (image->PixelType() != ttShort) {
      return dng_error_unsupported_dng;
    }
    dng_rect bounds = image->Bounds();
    uint32 w = static_cast<uint32>(bounds.r - bounds.l);
    uint32 h = static_cast<uint32>(bounds.b - bounds.t);
    uint32 planes = image->Planes();
    size_t count = static_cast<size_t>(w) * static_cast<size_t>(h) * static_cast<size_t>(planes);
    uint16_t *buffer = static_cast<uint16_t *>(malloc(count * sizeof(uint16_t)));
    if (buffer == nullptr) {
      return dng_error_memory;
    }
    dng_pixel_buffer pb;
    pb.fArea = bounds;
    pb.fPlane = 0;
    pb.fPlanes = planes;
    pb.fRowStep = static_cast<int32>(static_cast<size_t>(w) * planes);
    pb.fColStep = static_cast<int32>(planes);
    pb.fPlaneStep = 1;
    pb.fPixelType = ttShort;
    pb.fPixelSize = static_cast<uint32>(sizeof(uint16_t));
    pb.fData = buffer;
    image->Get(pb);
    *out_w = w;
    *out_h = h;
    *out_planes = planes;
    *out_data = buffer;
    *out_len = count;
    return dng_error_none;
  } catch (const dng_exception &except) {
    return except.ErrorCode();
  } catch (...) {
    return dng_error_unknown;
  }
}

// Releases a buffer returned by `gdng_read_raw`.
extern "C" void gdng_free(uint16_t *data) { free(data); }
