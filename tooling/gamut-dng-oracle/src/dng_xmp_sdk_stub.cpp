// A no-op replacement for the Adobe DNG SDK's `dng_xmp_sdk` (source/dng_xmp_sdk.cpp), which is the
// SDK's bridge to the Adobe XMP Toolkit (`#include "XMP.incl_cpp"`). No Linux XMP toolkit ships in
// the SDK bundle, so we build the SDK with XMP enabled (`qDNGUseXMP=1`, keeping `dng_metadata` and
// `dng_xmp` complete and the SDK source compiling cleanly) but exclude `dng_xmp_sdk.cpp` and define
// its methods here as no-ops. `HasMeta()` always returns false, so every XMP read/serialize path
// the SDK takes collapses to "no XMP present" — which is all the gamut-dng oracle needs (it
// validates pixel/structure conformance, not XMP content).
//
// Signatures must match `dng_xmp_sdk.h` exactly (these are mangled C++ symbols); the bodies are
// trivial. `qDNGXMPDocOps`/`qDNGXMPFiles` are disabled in the build, so the DocOps methods are not
// part of the class here.

#include "dng_xmp_sdk.h"

#include "dng_auto_ptr.h"
#include "dng_host.h"
#include "dng_local_string.h"
#include "dng_memory.h"
#include "dng_string.h"
#include "dng_string_list.h"

// The XMP namespace URI constants `dng_xmp_sdk.h` declares `extern` and `dng_xmp_sdk.cpp` defines.
// Other SDK units reference them, so they must be defined here (verbatim from the SDK).
const char *XMP_NS_TIFF = "http://ns.adobe.com/tiff/1.0/";
const char *XMP_NS_EXIF = "http://ns.adobe.com/exif/1.0/";
const char *XMP_NS_EXIFEX = "http://cipa.jp/exif/1.0/";
const char *XMP_NS_PHOTOSHOP = "http://ns.adobe.com/photoshop/1.0/";
const char *XMP_NS_XAP = "http://ns.adobe.com/xap/1.0/";
const char *XMP_NS_XAP_RIGHTS = "http://ns.adobe.com/xap/1.0/rights/";
const char *XMP_NS_DC = "http://purl.org/dc/elements/1.1/";
const char *XMP_NS_DC_TERMS = "http://purl.org/dc/terms/";
const char *XMP_NS_XMP_NOTE = "http://ns.adobe.com/xmp/note/";
const char *XMP_NS_MM = "http://ns.adobe.com/xap/1.0/mm/";
const char *XMP_NS_CRS = "http://ns.adobe.com/camera-raw-settings/1.0/";
const char *XMP_NS_CRSS = "http://ns.adobe.com/camera-raw-saved-settings/1.0/";
const char *XMP_NS_CRD = "http://ns.adobe.com/camera-raw-defaults/1.0/";
const char *XMP_NS_CRLCP = "http://ns.adobe.com/camera-raw-embedded-lens-profile/1.0/";
const char *XMP_NS_VFS = "http://ns.adobe.com/video-foundation-settings/1.0/";
const char *XMP_NS_LR = "http://ns.adobe.com/lightroom/1.0/";
const char *XMP_NS_LCP = "http://ns.adobe.com/photoshop/1.0/camera-profile";
const char *XMP_NS_AUX = "http://ns.adobe.com/exif/1.0/aux/";
const char *XMP_NS_IPTC = "http://iptc.org/std/Iptc4xmpCore/1.0/xmlns/";
const char *XMP_NS_IPTC_EXT = "http://iptc.org/std/Iptc4xmpExt/2008-02-29/";
const char *XMP_NS_PLUS = "http://ns.useplus.org/ldf/xmp/1.0/";
const char *XMP_NS_CRX = "http://ns.adobe.com/lightroom-settings-experimental/1.0/";
const char *XMP_NS_DNG = "http://ns.adobe.com/dng/1.0/";
const char *XMP_NS_PANO = "http://ns.adobe.com/photoshop/1.0/panorama-profile";
const char *XMP_NS_GPANO = "http://ns.google.com/photos/1.0/panorama/";
const char *XMP_NS_REGIONS = "http://www.metadataworkinggroup.com/schemas/regions/";
const char *XMP_NS_HDRGM = "http://ns.adobe.com/hdr-gain-map/1.0/";
const char *XMP_NS_HDR_META = "http://ns.adobe.com/hdr-metadata/1.0/";
const char *XMP_NS_APPLE_HDRGM = "http://ns.apple.com/HDRGainMap/1.0/";
const char *XMP_NS_APPLE_PIXELDATA = "http://ns.apple.com/pixeldatainfo/1.0/";

dng_xmp_sdk::dng_xmp_sdk() : fPrivate(nullptr) {}

dng_xmp_sdk::dng_xmp_sdk(const dng_xmp_sdk &) : fPrivate(nullptr) {}

dng_xmp_sdk::~dng_xmp_sdk() {}

void dng_xmp_sdk::InitializeSDK(dng_xmp_namespace *, const char *) {}

void dng_xmp_sdk::TerminateSDK() {}

bool dng_xmp_sdk::HasMeta() const { return false; }

void *dng_xmp_sdk::GetPrivateMeta() { return nullptr; }

void dng_xmp_sdk::Parse(dng_host &, const char *, uint32) {}

bool dng_xmp_sdk::Exists(const char *, const char *) const { return false; }

void dng_xmp_sdk::AppendArrayItem(const char *, const char *, const char *, bool, bool) {}

int32 dng_xmp_sdk::CountArrayItems(const char *, const char *) const { return 0; }

bool dng_xmp_sdk::HasNameSpace(const char *) const { return false; }

void dng_xmp_sdk::Remove(const char *, const char *) {}

void dng_xmp_sdk::RemoveProperties(const char *) {}

bool dng_xmp_sdk::IsEmptyString(const char *, const char *) { return true; }

bool dng_xmp_sdk::IsEmptyArray(const char *, const char *) { return true; }

void dng_xmp_sdk::ComposeArrayItemPath(const char *, const char *, int32, dng_string &) const {}

void dng_xmp_sdk::ComposeStructFieldPath(const char *, const char *, const char *, const char *,
                                         dng_string &) const {}

bool dng_xmp_sdk::GetNamespacePrefix(const char *, dng_string &) const { return false; }

bool dng_xmp_sdk::GetString(const char *, const char *, dng_string &) const { return false; }

void dng_xmp_sdk::ValidateStringList(const char *, const char *) {}

bool dng_xmp_sdk::GetStringList(const char *, const char *, dng_string_list &,
                                dng_abort_sniffer *) const {
  return false;
}

bool dng_xmp_sdk::GetAltLangDefault(const char *, const char *, dng_string &, bool) const {
  return false;
}

bool dng_xmp_sdk::GetLocalString(const char *, const char *, dng_local_string &) const {
  return false;
}

bool dng_xmp_sdk::GetStructField(const char *, const char *, const char *, const char *,
                                 dng_string &) const {
  return false;
}

void dng_xmp_sdk::Set(const char *, const char *, const char *) {}

void dng_xmp_sdk::SetString(const char *, const char *, const dng_string &) {}

void dng_xmp_sdk::SetStringList(const char *, const char *, const dng_string_list &, bool) {}

void dng_xmp_sdk::SetAltLangDefault(const char *, const char *, const dng_string &) {}

void dng_xmp_sdk::SetLocalString(const char *, const char *, const dng_local_string &) {}

void dng_xmp_sdk::SetStructField(const char *, const char *, const char *, const char *,
                                 const char *) {}

void dng_xmp_sdk::DeleteStructField(const char *, const char *, const char *, const char *) {}

dng_memory_block *dng_xmp_sdk::Serialize(dng_memory_allocator &, bool, uint32, uint32, bool,
                                         bool) const {
  return nullptr;
}

void dng_xmp_sdk::PackageForJPEG(dng_memory_allocator &, AutoPtr<dng_memory_block> &,
                                 AutoPtr<dng_memory_block> &, dng_string &) const {}

void dng_xmp_sdk::MergeFromJPEG(const dng_xmp_sdk *) {}

void dng_xmp_sdk::ReplaceXMP(dng_xmp_sdk *) {}

bool dng_xmp_sdk::IteratePaths(IteratePathsCallback *, void *, const char *, const char *, bool) {
  return false;
}

void dng_xmp_sdk::DuplicateSubtree(const dng_xmp_sdk &, const char *, const char *, const char *,
                                   const char *) {}

void dng_xmp_sdk::ClearMeta() {}

void dng_xmp_sdk::MakeMeta() {}

void dng_xmp_sdk::NeedMeta() {}

dng_xmp_sdk &dng_xmp_sdk::operator=(const dng_xmp_sdk &) { return *this; }
