// `extern "C"` shim over exiv2's IPTC-IIM and Photoshop IRB APIs.
//
// exiv2 has no C API, so this wraps the C++ entry points gamut-iptc's differential tests need:
// IptcParser::decode/encode (the IIM dataset stream) and Photoshop::locateIptcIrb (the 0x0404
// resource inside an 8BIM stream). XMP is disabled in the build, so this shim deliberately covers
// only the legacy binary carrier. Every entry point catches C++ exceptions and reports them as
// error codes so hostile input can never unwind across the FFI boundary.

#include <exiv2/iptc.hpp>
#include <exiv2/photoshop.hpp>
#include <exiv2/types.hpp>

#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <iterator>

using Exiv2::IptcData;
using Exiv2::IptcParser;
using Exiv2::Photoshop;

extern "C" {

// Number of IIM datasets exiv2 decodes from a dataset stream; -1 if exiv2 rejects the stream.
int64_t gex_iim_count(const uint8_t* data, size_t len) {
  try {
    IptcData iptc;
    if (IptcParser::decode(iptc, data, len) != 0) {
      return -1;
    }
    return static_cast<int64_t>(std::distance(iptc.begin(), iptc.end()));
  } catch (...) {
    return -1;
  }
}

// The `index`-th decoded dataset's record, tag, and raw value octets. Returns 0 on success and
// allocates `*out` (free with gex_free); nonzero on decode error or out-of-range index.
int gex_iim_dataset(const uint8_t* data, size_t len, size_t index, uint16_t* record, uint16_t* tag,
                    uint8_t** out, size_t* out_len) {
  try {
    IptcData iptc;
    if (IptcParser::decode(iptc, data, len) != 0) {
      return 1;
    }
    size_t i = 0;
    for (auto it = iptc.begin(); it != iptc.end(); ++it, ++i) {
      if (i == index) {
        *record = it->record();
        *tag = it->tag();
        size_t n = it->size();
        auto* buf = static_cast<uint8_t*>(std::malloc(n ? n : 1));
        if (buf == nullptr) {
          return 3;
        }
        it->copy(buf, Exiv2::bigEndian);
        *out = buf;
        *out_len = n;
        return 0;
      }
    }
    return 2;  // index out of range
  } catch (...) {
    return 1;
  }
}

// Decode then re-encode `data` with exiv2 (the reference IIM round-trip). Returns 0 and allocates
// `*out` (free with gex_free); nonzero on error.
int gex_iim_reencode(const uint8_t* data, size_t len, uint8_t** out, size_t* out_len) {
  try {
    IptcData iptc;
    if (IptcParser::decode(iptc, data, len) != 0) {
      return 1;
    }
    Exiv2::DataBuf buf = IptcParser::encode(iptc);
    size_t n = buf.size();
    auto* o = static_cast<uint8_t*>(std::malloc(n ? n : 1));
    if (o == nullptr) {
      return 2;
    }
    if (n) {
      std::memcpy(o, buf.c_data(), n);
    }
    *out = o;
    *out_len = n;
    return 0;
  } catch (...) {
    return 1;
  }
}

// Locate the IPTC (0x0404) IIM payload within a Photoshop 8BIM stream. Returns 0 and allocates
// `*out` (free with gex_free); nonzero if absent or the stream is invalid.
int gex_irb_iptc(const uint8_t* data, size_t len, uint8_t** out, size_t* out_len) {
  try {
    const Exiv2::byte* record = nullptr;
    uint32_t size_hdr = 0;
    uint32_t size_data = 0;
    if (Photoshop::locateIptcIrb(data, len, &record, size_hdr, size_data) != 0 || record == nullptr) {
      return 1;
    }
    auto* o = static_cast<uint8_t*>(std::malloc(size_data ? size_data : 1));
    if (o == nullptr) {
      return 2;
    }
    if (size_data) {
      std::memcpy(o, record + size_hdr, size_data);
    }
    *out = o;
    *out_len = size_data;
    return 0;
  } catch (...) {
    return 1;
  }
}

void gex_free(uint8_t* p) {
  std::free(p);
}

}  // extern "C"
