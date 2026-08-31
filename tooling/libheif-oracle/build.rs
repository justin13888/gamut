//! Builds the HEIF decode/encode reference stack — `libde265` (HEVC decoder), `kvazaar` (HEVC
//! encoder) and `libheif` (ISO/IEC 23008-12 container) — from the `third_party/` submodules into
//! `OUT_DIR`, then generates FFI bindings for libheif's and libde265's public APIs.
//!
//! Three hermetic cmake stages (no system HEIF stack is ever consulted):
//!   1. build + install a static `libde265.a` (decoder library only — the `dec265`/`sherlock265`
//!      tools and SDL are disabled);
//!   2. build + install a static `libkvazaar.a` (library only, no CLI binary/tests). kvazaar's
//!      CMake build compiles its portable C paths, so it needs no yasm/nasm;
//!   3. build + install a static `libheif.a` with the built-in libde265 decoder and kvazaar
//!      encoder (`ENABLE_PLUGIN_LOADING=OFF` ⇒ codecs are compiled in, not dlopen'd), every other
//!      codec/plugin/example/test off. libheif discovers stages 1–2 through
//!      `CMAKE_PREFIX_PATH` + `PKG_CONFIG_PATH` pointed at their `OUT_DIR` install prefixes.
//!
//! Every install pins `CMAKE_INSTALL_LIBDIR=lib` so the archives and `.pc` files land at a
//! deterministic path across distros (no `lib64` split). Everything lands under `OUT_DIR`, so
//! `cargo clean` fully resets the build.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(env("CARGO_MANIFEST_DIR"));
    let out = PathBuf::from(env("OUT_DIR"));
    let third_party = manifest.join("../../third_party");
    let de265_src = third_party.join("libde265");
    let kvazaar_src = third_party.join("kvazaar");
    let heif_src = third_party.join("libheif");

    assert!(
        de265_src.join("CMakeLists.txt").exists()
            && kvazaar_src.join("CMakeLists.txt").exists()
            && heif_src.join("CMakeLists.txt").exists(),
        "vendored libde265/kvazaar/libheif not found under {} — run `git submodule update --init --recursive`",
        third_party.display()
    );

    // ---- Stage 1: libde265 — static decoder library, no tools/SDL. ------------------------------
    let de265_install = out.join("de265-install");
    cmake_build_install(
        &de265_src,
        &out.join("de265-build"),
        &de265_install,
        &[
            "-DBUILD_SHARED_LIBS=OFF",
            "-DCMAKE_POSITION_INDEPENDENT_CODE=ON",
            "-DENABLE_SDL=OFF", // no SDL dependency for the (unbuilt) dec265 viewer
            "-DENABLE_DECODER=OFF", // skip the dec265 CLI tool; the decode API stays in the lib
            "-DENABLE_ENCODER=OFF", // no enc265
            "-DENABLE_SHERLOCK265=OFF",
        ],
    );

    // ---- Stage 2: kvazaar — static encoder library only. ----------------------------------------
    let kvazaar_install = out.join("kvazaar-install");
    cmake_build_install(
        &kvazaar_src,
        &out.join("kvazaar-build"),
        &kvazaar_install,
        &[
            "-DBUILD_SHARED_LIBS=OFF",
            "-DCMAKE_POSITION_INDEPENDENT_CODE=ON",
            "-DBUILD_KVAZAAR_BINARY=OFF", // library only, no CLI
            "-DBUILD_TESTS=OFF",
        ],
    );

    // ---- Stage 3: libheif — static, built-in libde265 decode + kvazaar encode, all else off. ----
    let heif_install = out.join("heif-install");
    let prefix_path = format!(
        "{};{}",
        path_str(&de265_install),
        path_str(&kvazaar_install)
    );
    let pkg_config_path = std::env::join_paths([
        de265_install.join("lib/pkgconfig"),
        kvazaar_install.join("lib/pkgconfig"),
    ])
    .expect("join PKG_CONFIG_PATH");
    cmake_build_install_env(
        &heif_src,
        &out.join("heif-build"),
        &heif_install,
        &[
            "-DBUILD_SHARED_LIBS=OFF",
            "-DCMAKE_POSITION_INDEPENDENT_CODE=ON",
            "-DENABLE_PLUGIN_LOADING=OFF", // compile codecs in statically, no dlopen
            // Enabled codecs: the built-in libde265 decoder and kvazaar encoder.
            "-DWITH_LIBDE265=ON",
            "-DWITH_KVAZAAR=ON",
            // Every other codec/plugin off — keep the build hermetic and minimal.
            "-DWITH_X265=OFF",
            "-DWITH_X264=OFF",
            "-DWITH_OpenH264_DECODER=OFF",
            "-DWITH_AOM_DECODER=OFF",
            "-DWITH_AOM_ENCODER=OFF",
            "-DWITH_DAV1D=OFF",
            "-DWITH_SvtEnc=OFF",
            "-DWITH_RAV1E=OFF",
            "-DWITH_JPEG_DECODER=OFF",
            "-DWITH_JPEG_ENCODER=OFF",
            "-DWITH_OpenJPEG_DECODER=OFF",
            "-DWITH_OpenJPEG_ENCODER=OFF",
            "-DWITH_OPENJPH_ENCODER=OFF",
            "-DWITH_FFMPEG_DECODER=OFF",
            "-DWITH_UVG266=OFF",
            "-DWITH_VVDEC=OFF",
            "-DWITH_VVENC=OFF",
            "-DWITH_UNCOMPRESSED_CODEC=OFF",
            "-DWITH_LIBSHARPYUV=OFF",
            "-DWITH_HEADER_COMPRESSION=OFF",
            // No examples/tools/tests/docs/bindings.
            "-DWITH_EXAMPLES=OFF",
            "-DWITH_EXAMPLE_HEIF_THUMB=OFF",
            "-DWITH_EXAMPLE_HEIF_VIEW=OFF",
            "-DWITH_GDK_PIXBUF=OFF",
            "-DBUILD_TESTING=OFF",
            "-DBUILD_DOCUMENTATION=OFF",
            &format!("-DCMAKE_PREFIX_PATH={prefix_path}"),
        ],
        &[("PKG_CONFIG_PATH", pkg_config_path.as_os_str())],
    );

    // ---- Link. Order matters for a static link: heif → de265 → kvazaar, then the C++ runtime. ---
    for install in [&heif_install, &de265_install, &kvazaar_install] {
        println!(
            "cargo:rustc-link-search=native={}",
            path_str(&install.join("lib"))
        );
    }
    println!("cargo:rustc-link-lib=static=heif");
    println!("cargo:rustc-link-lib=static=de265");
    println!("cargo:rustc-link-lib=static=kvazaar");
    // libheif and libde265 are C++; pull in the C++ runtime and the pthread/m the codecs use.
    if env("CARGO_CFG_TARGET_OS") == "macos" {
        println!("cargo:rustc-link-lib=dylib=c++");
    } else {
        println!("cargo:rustc-link-lib=dylib=stdc++");
        println!("cargo:rustc-link-lib=dylib=m");
        println!("cargo:rustc-link-lib=dylib=pthread");
    }

    // ---- Bindings over the installed public headers (heif_version.h is generated at install). ---
    let include = heif_install.join("include");
    let bindings = bindgen::Builder::default()
        .header(path_str(&manifest.join("wrapper.h")))
        .clang_arg(format!("-I{}", path_str(&include)))
        .clang_arg(format!("-I{}", path_str(&de265_install.join("include"))))
        .allowlist_function("heif_.*")
        .allowlist_type("heif_.*")
        .allowlist_var("heif_.*")
        .allowlist_function("de265_.*")
        .allowlist_type("de265_.*")
        .allowlist_var("(de265_.*|DE265_.*)")
        // Keep enum constants spelled exactly as in C (`DE265_OK`, `heif_chroma_interleaved_RGBA`).
        .prepend_enum_name(false)
        .layout_tests(false)
        .generate()
        .expect("generate libheif/libde265 FFI bindings");
    bindings
        .write_to_file(out.join("bindings.rs"))
        .expect("write bindings");

    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=build.rs");
    println!(
        "cargo:rerun-if-changed={}",
        path_str(&heif_src.join("libheif/api/libheif/heif.h"))
    );
    println!(
        "cargo:rerun-if-changed={}",
        path_str(&de265_src.join("libde265/de265.h"))
    );
}

/// cmake-configure (once), build, and `install` a project into `install_prefix`. Skips the
/// configure step if the build dir is already configured, so rebuilds are incremental.
fn cmake_build_install(src: &Path, build: &Path, install_prefix: &Path, extra: &[&str]) {
    cmake_build_install_env(src, build, install_prefix, extra, &[]);
}

/// As [`cmake_build_install`], but sets extra environment variables (e.g. `PKG_CONFIG_PATH`) on
/// both the configure and build invocations.
fn cmake_build_install_env(
    src: &Path,
    build: &Path,
    install_prefix: &Path,
    extra: &[&str],
    envs: &[(&str, &std::ffi::OsStr)],
) {
    if !build.join("CMakeCache.txt").exists() {
        let mut cmd = Command::new("cmake");
        cmd.arg("-S")
            .arg(src)
            .arg("-B")
            .arg(build)
            .args([
                "-DCMAKE_BUILD_TYPE=Release",
                // Pin the libdir so archives and `.pc` files land at a deterministic path.
                "-DCMAKE_INSTALL_LIBDIR=lib",
            ])
            .args(extra)
            .arg(format!(
                "-DCMAKE_INSTALL_PREFIX={}",
                path_str(install_prefix)
            ));
        for (k, v) in envs {
            cmd.env(k, v);
        }
        run(&mut cmd);
    }
    // `--parallel` takes an explicit count. Bare, it becomes `make -j` with no limit *and*
    // overrides CMAKE_BUILD_PARALLEL_LEVEL, so the shared dial would be silently ignored — and
    // this helper drives three C++ trees (libde265, kvazaar, libheif).
    let mut cmd = Command::new("cmake");
    cmd.arg("--build").arg(build).args([
        "--config",
        "Release",
        "--target",
        "install",
        "--parallel",
        &build_env::build_parallelism().to_string(),
    ]);
    for (k, v) in envs {
        cmd.env(k, v);
    }
    run(&mut cmd);
}

/// Reads a required build-time env var, panicking (this is a build script) if absent.
fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("missing build env var {key}"))
}

/// `Path` → `&str`, panicking on non-UTF-8 paths (none exist in this build tree).
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
