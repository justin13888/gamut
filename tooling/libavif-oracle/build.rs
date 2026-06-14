//! Builds a static `libavif` (with the vendored dav1d as its only decoder) from the
//! `third_party/libavif` and `third_party/dav1d` submodules, and generates FFI bindings
//! for its public decode API.
//!
//! Two stages, both hermetic (no system dav1d/libavif is ever consulted):
//!   1. meson-build + install the vendored dav1d into an `OUT_DIR` prefix, exposing a
//!      `dav1d.pc` so libavif's `AVIF_CODEC_DAV1D=SYSTEM` finds exactly that build;
//!   2. cmake-build a static `libavif.a` against that prefix.
//!
//! Everything lands under `OUT_DIR`, so `cargo clean` fully resets the build.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env("OUT_DIR"));
    let third_party = manifest_dir.join("../../third_party");
    let dav1d_src = third_party.join("dav1d");
    let avif_src = third_party.join("libavif");

    assert!(
        dav1d_src.join("meson.build").exists() && avif_src.join("CMakeLists.txt").exists(),
        "vendored dav1d/libavif not found under {} — run `git submodule update --init --recursive`",
        third_party.display()
    );

    // ---- Stage 1: build + install vendored dav1d into an OUT_DIR prefix. -------------------
    // dav1d's meson does `find_program('nasm')` to assemble its x86 SIMD; `path` is the
    // process PATH with a vendored nasm prepended (unchanged on non-x86). Only this stage
    // needs it — the stage-2 cmake build of libavif has no nasm dependency (libavif's own
    // asm is gated on the SVT/AVM codecs, which we disable).
    let path = path_with_nasm();

    let dav1d_build = out_dir.join("dav1d-build");
    let dav1d_prefix = out_dir.join("dav1d-prefix");
    if !dav1d_build.join("build.ninja").exists() {
        run(Command::new("meson")
            .env("PATH", &path)
            .arg("setup")
            .arg(&dav1d_build)
            .arg(&dav1d_src)
            .arg(format!("--prefix={}", path_str(&dav1d_prefix)))
            // Pin the libdir to `lib` (not the Debian multiarch `lib/<triple>` or `lib64`) so the
            // installed `libdav1d.a` and `dav1d.pc` land at a path we can hand to rustc and
            // pkg-config deterministically across platforms.
            .arg("--libdir=lib")
            .args([
                "--default-library=static",
                "--buildtype=release",
                "-Denable_tools=false",
                "-Denable_tests=false",
                "-Denable_docs=false",
            ]));
    }
    // `meson install` compiles the asm objects, so it also needs the vendored nasm on PATH.
    run(Command::new("meson")
        .env("PATH", &path)
        .arg("install")
        .arg("-C")
        .arg(&dav1d_build));
    let pkgconfig = dav1d_prefix.join("lib").join("pkgconfig");

    // ---- Stage 2: cmake-build a static libavif against that dav1d. -------------------------
    let avif_build = out_dir.join("avif-build");
    if !avif_build.join("CMakeCache.txt").exists() {
        run(Command::new("cmake")
            .env("PKG_CONFIG_PATH", &pkgconfig)
            .arg("-S")
            .arg(&avif_src)
            .arg("-B")
            .arg(&avif_build)
            .args([
                "-DCMAKE_BUILD_TYPE=Release",
                "-DBUILD_SHARED_LIBS=OFF",
                "-DAVIF_CODEC_DAV1D=SYSTEM",
                "-DAVIF_LIBYUV=OFF",
                "-DAVIF_LIBSHARPYUV=OFF",
                "-DAVIF_JPEG=OFF",
                "-DAVIF_ZLIBPNG=OFF",
                "-DAVIF_BUILD_APPS=OFF",
                "-DAVIF_BUILD_TESTS=OFF",
            ])
            .arg(format!("-DCMAKE_PREFIX_PATH={}", path_str(&dav1d_prefix))));
    }
    run(Command::new("cmake")
        .env("PKG_CONFIG_PATH", &pkgconfig)
        .arg("--build")
        .arg(&avif_build)
        .arg("--parallel"));

    // ---- Link. The `avif_static` target merges dav1d into `libavif.a`, but we also offer the
    // installed `libdav1d.a` so platforms whose merge is a no-op still resolve dav1d. -------
    println!("cargo:rustc-link-search=native={}", path_str(&avif_build));
    println!(
        "cargo:rustc-link-search=native={}",
        path_str(&dav1d_prefix.join("lib"))
    );
    println!("cargo:rustc-link-lib=static=avif");
    println!("cargo:rustc-link-lib=static=dav1d");
    if env("CARGO_CFG_TARGET_OS") != "macos" {
        println!("cargo:rustc-link-lib=dylib=m");
        println!("cargo:rustc-link-lib=dylib=pthread");
    }

    // ---- Bindings. -------------------------------------------------------------------------
    let bindings = bindgen::Builder::default()
        .header(path_str(&manifest_dir.join("wrapper.h")))
        .clang_arg(format!("-I{}", path_str(&avif_src.join("include"))))
        .allowlist_function("avif.*")
        .allowlist_type("avif.*")
        .allowlist_var("AVIF_.*")
        .prepend_enum_name(false)
        .layout_tests(false)
        .generate()
        .expect("generate libavif FFI bindings");
    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("write libavif bindings");

    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=build.rs");
    // Re-run when the vendored nasm tarball is swapped (e.g. a version bump).
    println!("cargo:rerun-if-changed={}", nasm_vendor::tarball_path());
    println!(
        "cargo:rerun-if-changed={}",
        path_str(&avif_src.join("include/avif/avif.h"))
    );
}

/// The process `PATH` with the vendored nasm directory prepended, so meson's
/// `find_program('nasm')` resolves to it. On arches/platforms where no vendored nasm is
/// built (non-x86, non-Unix), returns the unchanged `PATH`.
fn path_with_nasm() -> std::ffi::OsString {
    let base = std::env::var_os("PATH").unwrap_or_default();
    match nasm_vendor::ensure_nasm() {
        Some(dir) => std::env::join_paths(std::iter::once(dir).chain(std::env::split_paths(&base)))
            .expect("join PATH with vendored nasm dir"),
        None => base,
    }
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
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn {cmd:?}: {e}"));
    assert!(status.success(), "command failed ({status}): {cmd:?}");
}
