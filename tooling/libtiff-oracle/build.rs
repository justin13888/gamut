//! Builds a static `libtiff` from the `third_party/libtiff` submodule and generates FFI bindings
//! for its public API.
//!
//! A vendored static zlib enables Deflate; every other optional codec (JPEG, JBIG, LZMA, ZSTD,
//! WebP, LERC, PixarLog) is disabled. Everything lands under `OUT_DIR`, so the oracle remains
//! hermetic and `cargo clean` fully resets the build.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env("OUT_DIR"));
    let zlib_submodule = manifest_dir.join("../../third_party/zlib");
    let src = manifest_dir.join("../../third_party/libtiff");

    for (name, source) in [("zlib", &zlib_submodule), ("libtiff", &src)] {
        assert!(
            source.join("CMakeLists.txt").exists(),
            "vendored {name} not found under {} — run `git submodule update --init --recursive`",
            source.display()
        );
    }

    // zlib's CMake deletes its source-tree zconf.h even for an out-of-source build. Build a private
    // copy so the submodule remains pristine.
    let zlib_src = out_dir.join("zlib-src");
    if !zlib_src.join("CMakeLists.txt").exists() {
        copy_dir(&zlib_submodule, &zlib_src);
    }
    let zlib_build = out_dir.join("zlib-build");
    let zlib_prefix = out_dir.join("zlib-prefix");
    if !zlib_build.join("CMakeCache.txt").exists() {
        run(Command::new("cmake")
            .arg("-S")
            .arg(&zlib_src)
            .arg("-B")
            .arg(&zlib_build)
            .args([
                "-DCMAKE_BUILD_TYPE=Release",
                "-DCMAKE_POSITION_INDEPENDENT_CODE=ON",
                "-DZLIB_BUILD_EXAMPLES=OFF",
                &format!("-DCMAKE_INSTALL_PREFIX={}", path_str(&zlib_prefix)),
                "-DCMAKE_INSTALL_LIBDIR=lib",
            ]));
    }
    run(Command::new("cmake").arg("--build").arg(&zlib_build).args([
        "--config",
        "Release",
        "--parallel",
        "--target",
        "install",
    ]));

    // ---- CMake-build a static libtiff with only the vendored zlib codec enabled. ------------
    let build = out_dir.join("libtiff-build");
    // Always configure: changing the oracle's codec matrix must update an existing OUT_DIR cache.
    run(Command::new("cmake")
        .arg("-S")
        .arg(&src)
        .arg("-B")
        .arg(&build)
        .args([
            "-DCMAKE_BUILD_TYPE=Release",
            "-DBUILD_SHARED_LIBS=OFF",
            // Compile the static archive position-independent so it links into Rust test
            // binaries on PIE-by-default toolchains (e.g. Fedora). libtiff's own CMake does
            // not enable PIC (unlike the vendored libavif), so we set it here; it is a no-op
            // on platforms that don't produce PIE executables.
            "-DCMAKE_POSITION_INDEPENDENT_CODE=ON",
            // Deflate uses the vendored static zlib. Disable every other optional codec/dependency
            // and auxiliary target so the archive remains self-contained.
            "-Dzlib=ON",
            &format!("-DZLIB_ROOT={}", path_str(&zlib_prefix)),
            &format!("-DCMAKE_PREFIX_PATH={}", path_str(&zlib_prefix)),
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
    println!(
        "cargo:rustc-link-search=native={}",
        path_str(&zlib_prefix.join("lib"))
    );
    println!("cargo:rustc-link-lib=static=tiff");
    println!("cargo:rustc-link-lib=static=z");
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

/// Recursively copies `src` into `dst` (skipping `.git`).
fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap_or_else(|e| panic!("create {}: {e}", dst.display()));
    for entry in fs::read_dir(src)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", src.display()))
        .flatten()
    {
        let path = entry.path();
        if path.file_name().is_some_and(|name| name == ".git") {
            continue;
        }
        let target = dst.join(path.file_name().expect("dir entry has a name"));
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            fs::copy(&path, &target).unwrap_or_else(|e| panic!("copy {}: {e}", path.display()));
        }
    }
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
    // Normalise any compiler-launcher env (sccache/ccache) into the single position cmake
    // defines for it, so this build is hermetic to exactly what it configures. See the
    // `build-env` crate docs; `GAMUT_BUILD_KEEP_ENV=1` opts out.
    build_env::sanitize(cmd);
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn {cmd:?}: {e}"));
    assert!(status.success(), "command failed ({status}): {cmd:?}");
}
