//! Builds the Adobe DNG SDK 1.7.1 headless from the ZIP in `references/dng/` and links it behind a
//! small `extern "C"` shim.
//!
//! The SDK ships only Visual Studio / Xcode projects, so we compile its `source/*.cpp` directly
//! with the `cc` crate. Three external dependencies the 1.7.1 source assumes are absent on a
//! headless Linux box, so each is replaced by a stub that satisfies the compile/link without
//! changing what the oracle observes (it validates pixel/structure conformance, not metadata):
//!
//! - **Adobe XMP Toolkit** — XMP is kept *enabled* (`qDNGUseXMP=1`, so `dng_metadata`/`dng_xmp`
//!   stay complete and every SDK unit compiles), but its toolkit bridge `dng_xmp_sdk.cpp` is
//!   excluded and replaced by the no-op `src/dng_xmp_sdk_stub.cpp`. The XMP files/doc-ops layers
//!   are off.
//! - **libjpeg** — off (`qDNGUseLibJPEG=0`); lossless JPEG is the SDK's own codec.
//! - **libjxl** — wired in unconditionally by 1.7.1; satisfied by the link-only stubs in
//!   `src/jxl_stub.cpp` (plus generated export-header stubs; see [`write_jxl_export_stubs`]).
//!
//! Only the system `zlib` (`-lz`) is genuinely required, since the SDK includes `zlib.h`
//! unconditionally for its Deflate and big-table paths.
//!
//! Everything lands under `OUT_DIR`, so `cargo clean` fully resets the build. CI needs the ZIP
//! (committed under `references/dng/`) and a C++ toolchain + zlib headers; nothing else.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

/// Source files to exclude from the compile: the `main()` driver, the XMP-toolkit bridge (which
/// needs the absent Adobe XMP Toolkit — replaced by the no-op `src/dng_xmp_sdk_stub.cpp`), and the
/// lossless-JPEG shared unit (it is `#include`d by `dng_lossless_jpeg.cpp`, so compiling it
/// separately would duplicate symbols).
const EXCLUDE: &[&str] = &[
    "dng_validate.cpp",
    "dng_xmp_sdk.cpp",
    "dng_lossless_jpeg_shared.cpp",
];

fn main() {
    let manifest = PathBuf::from(env("CARGO_MANIFEST_DIR"));
    let out = PathBuf::from(env("OUT_DIR"));
    let zip = manifest.join("../../references/dng/dng_sdk_1_7_1_2611_20260609.zip");

    assert!(
        zip.exists(),
        "DNG SDK ZIP not found at {} — it is committed under references/dng/; ensure a full checkout",
        zip.display()
    );

    // ---- Extract the SDK once (guarded by a marker), into OUT_DIR. ------------------------------
    let sdk = out.join("dng_sdk_extracted");
    let base = sdk.join("dng_sdk_1_7_1");
    let source = base.join("dng_sdk").join("source");
    let jxl_include = base
        .join("libjxl")
        .join("libjxl")
        .join("lib")
        .join("include");
    let marker = sdk.join(".extracted");
    if !marker.exists() {
        extract_zip(&zip, &sdk);
        fs::write(&marker, b"ok").expect("write extraction marker");
    }
    assert!(
        source.join("dng_negative.cpp").exists(),
        "extracted SDK source not found under {}",
        source.display()
    );

    // libjxl's public headers `#include` CMake-generated export-macro headers that are not in the
    // source tree; provide empty-macro stubs (static build, no symbol visibility decoration).
    let jxl_shim = out.join("jxl_shim");
    write_jxl_export_stubs(&jxl_shim.join("jxl"));

    // ---- Compile the SDK + shim + libjxl stubs into one static archive. -------------------------
    let mut build = cc::Build::new();
    build.cpp(true).std("c++17");
    build.include(&source);
    build.include(&jxl_shim);
    build.include(&jxl_include);
    // Headless Linux, little-endian. XMP is *enabled* (so `dng_metadata`/`dng_xmp` stay complete
    // and the SDK source compiles cleanly) but its toolkit bridge is the no-op
    // `src/dng_xmp_sdk_stub.cpp`; the XMP files/doc-ops layers are off (nothing references them).
    // libjpeg and threading are off (lossless JPEG is the SDK's own codec). The
    // platform/endianness/64-bit macros resolve automatically once qLinux=1 is set.
    for (k, v) in [
        ("qLinux", "1"),
        ("qDNGUseXMP", "1"),
        ("qDNGXMPFiles", "0"),
        ("qDNGXMPDocOps", "0"),
        ("qDNGUseLibJPEG", "0"),
        ("qDNGThreadSafe", "0"),
        ("qDNGValidate", "0"),
        ("qDNGValidateTarget", "0"),
    ] {
        build.define(k, v);
    }
    // The vendored SDK is warning-heavy; it is reference code we do not own, so silence warnings
    // rather than treat them as signal.
    build.flag_if_supported("-w");

    let exclude: HashSet<&str> = EXCLUDE.iter().copied().collect();
    let mut count = 0;
    for entry in fs::read_dir(&source).expect("read SDK source dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) == Some("cpp") {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            if !exclude.contains(name) {
                build.file(&path);
                count += 1;
            }
        }
    }
    assert!(
        count > 50,
        "expected the full SDK source set, found only {count} files"
    );

    build.file(manifest.join("src/oracle_shim.cpp"));
    build.file(manifest.join("src/jxl_stub.cpp"));
    build.file(manifest.join("src/dng_xmp_sdk_stub.cpp"));
    build.compile("dng_oracle");

    // The SDK includes <zlib.h> unconditionally (Deflate + big-table compression); link system z.
    println!("cargo:rustc-link-lib=dylib=z");

    println!("cargo:rerun-if-changed=src/oracle_shim.cpp");
    println!("cargo:rerun-if-changed=src/jxl_stub.cpp");
    println!("cargo:rerun-if-changed=src/dng_xmp_sdk_stub.cpp");
    println!("cargo:rerun-if-changed=build.rs");
}

/// Extracts every entry of the ZIP at `zip` under `dest`.
fn extract_zip(zip: &Path, dest: &Path) {
    let file = fs::File::open(zip).expect("open DNG SDK ZIP");
    let mut archive = zip::ZipArchive::new(file).expect("read DNG SDK ZIP");
    archive.extract(dest).expect("extract DNG SDK ZIP");
}

/// Writes empty-macro stubs for the CMake-generated libjxl export headers under `dir`.
fn write_jxl_export_stubs(dir: &Path) {
    fs::create_dir_all(dir).expect("create jxl export-stub dir");
    for (file, prefix) in [
        ("jxl_export.h", "JXL"),
        ("jxl_threads_export.h", "JXL_THREADS"),
        ("jxl_cms_export.h", "JXL_CMS"),
    ] {
        let content = format!(
            "#ifndef {p}_EXPORT_H\n#define {p}_EXPORT_H\n\
             #define {p}_EXPORT\n#define {p}_NO_EXPORT\n#define {p}_DEPRECATED\n\
             #define {p}_DEPRECATED_EXPORT\n#define {p}_DEPRECATED_NO_EXPORT\n#endif\n",
            p = prefix
        );
        fs::write(dir.join(file), content).expect("write jxl export stub");
    }
}

/// Reads a required build-time env var, panicking (this is a build script) if absent.
fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("missing build env var {key}"))
}
