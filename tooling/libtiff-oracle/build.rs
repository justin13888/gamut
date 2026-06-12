//! Builds a static `libtiff` from the `third_party/libtiff` submodule and generates FFI bindings
//! for its public API.
//!
//! All optional codecs (zlib/deflate, JPEG, JBIG, LZMA, ZSTD, WebP, LERC, PixarLog) are turned
//! off so the build is hermetic — it pulls in no system libraries — leaving libtiff's built-in
//! schemes: uncompressed, PackBits, LZW, and CCITT (Modified Huffman / T.4 / T.6). Everything
//! lands under `OUT_DIR`, so `cargo clean` fully resets the build.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env("OUT_DIR"));
    let src = manifest_dir.join("../../third_party/libtiff");

    assert!(
        src.join("CMakeLists.txt").exists(),
        "vendored libtiff not found under {} — run `git submodule update --init --recursive`",
        src.display()
    );

    // ---- CMake-build a static libtiff with every optional codec disabled. ------------------
    let build = out_dir.join("libtiff-build");
    if !build.join("CMakeCache.txt").exists() {
        run(Command::new("cmake")
            .arg("-S")
            .arg(&src)
            .arg("-B")
            .arg(&build)
            .args([
                "-DCMAKE_BUILD_TYPE=Release",
                "-DBUILD_SHARED_LIBS=OFF",
                // Disable every optional codec/dependency and the auxiliary build targets so the
                // static archive is self-contained.
                "-Dzlib=OFF",
                "-Dlibdeflate=OFF",
                "-Dpixarlog=OFF",
                "-Djpeg=OFF",
                "-Dold-jpeg=OFF",
                "-Djbig=OFF",
                "-Dlzma=OFF",
                "-Dzstd=OFF",
                "-Dwebp=OFF",
                "-Dlerc=OFF",
                "-Dcxx=OFF",
                "-Dtiff-tools=OFF",
                "-Dtiff-tests=OFF",
                "-Dtiff-contrib=OFF",
                "-Dtiff-docs=OFF",
            ]));
    }
    run(Command::new("cmake").arg("--build").arg(&build).args([
        "--config",
        "Release",
        "--parallel",
    ]));

    // ---- Link. ----------------------------------------------------------------------------
    println!(
        "cargo:rustc-link-search=native={}",
        path_str(&build.join("libtiff"))
    );
    println!("cargo:rustc-link-lib=static=tiff");
    if env("CARGO_CFG_TARGET_OS") != "macos" {
        println!("cargo:rustc-link-lib=dylib=m");
    }

    // ---- Bindings. The generated `tiffconf.h`/`tif_config.h` live in the build tree. -------
    let bindings = bindgen::Builder::default()
        .header(path_str(&manifest_dir.join("wrapper.h")))
        .clang_arg(format!("-I{}", path_str(&src.join("libtiff"))))
        .clang_arg(format!("-I{}", path_str(&build.join("libtiff"))))
        .allowlist_function("TIFF.*")
        .allowlist_type("TIFF.*")
        .allowlist_var("TIFFTAG_.*")
        .allowlist_var("PHOTOMETRIC_.*")
        .allowlist_var("COMPRESSION_.*")
        .allowlist_var("PLANARCONFIG_.*")
        .allowlist_var("SAMPLEFORMAT_.*")
        .allowlist_var("RESUNIT_.*")
        .allowlist_var("ORIENTATION_.*")
        .allowlist_var("FILLORDER_.*")
        .allowlist_var("EXTRASAMPLE_.*")
        .prepend_enum_name(false)
        .layout_tests(false)
        .generate()
        .expect("generate libtiff FFI bindings");
    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("write libtiff bindings");

    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=build.rs");
    println!(
        "cargo:rerun-if-changed={}",
        path_str(&src.join("libtiff/tiffio.h"))
    );
}

/// Reads a required build-time env var, panicking (this is a build script) if absent.
fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("missing build env var {key}"))
}

/// `Path` → `String`, panicking on non-UTF-8 paths (none exist in this build tree).
fn path_str(p: &Path) -> String {
    p.to_str()
        .unwrap_or_else(|| panic!("non-UTF-8 path: {}", p.display()))
        .to_string()
}

/// Runs a build subcommand, aborting the build with its output on failure.
fn run(cmd: &mut Command) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn {cmd:?}: {e}"));
    assert!(status.success(), "command failed ({status}): {cmd:?}");
}
