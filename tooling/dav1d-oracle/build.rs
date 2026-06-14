//! Builds a static `libdav1d` from the vendored `third_party/dav1d` submodule
//! (meson + ninja) and generates FFI bindings for its public decode API.
//!
//! The build is hermetic: it never looks for a system-installed dav1d. The static
//! archive and bindings land in `OUT_DIR`, so `cargo clean` fully resets them.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env("OUT_DIR"));
    let src = manifest_dir.join("../../third_party/dav1d");

    // Fail early with an actionable message if the submodule was not checked out.
    let meson_build = src.join("meson.build");
    assert!(
        meson_build.exists(),
        "vendored dav1d not found at {} — run `git submodule update --init --recursive`",
        src.display()
    );

    // Build/locate a vendored nasm before configuring: dav1d's meson does
    // `find_program('nasm')` to assemble its x86 SIMD. `path` is the process PATH with the
    // vendored nasm prepended (unchanged on non-x86, where dav1d needs no nasm).
    let path = path_with_nasm();

    let build_dir = out_dir.join("dav1d-build");
    // meson refuses to re-`setup` an already-configured directory; the presence of
    // `build.ninja` is its own marker that configuration succeeded, so on rebuilds we
    // skip straight to an incremental `ninja`.
    if !build_dir.join("build.ninja").exists() {
        run(Command::new("meson")
            .env("PATH", &path)
            .arg("setup")
            .arg(&build_dir)
            .arg(&src)
            .args([
                "--default-library=static",
                "--buildtype=release",
                "-Denable_tools=false",
                "-Denable_tests=false",
                "-Denable_docs=false",
            ]));
    }
    // meson bakes nasm's absolute path into `build.ninja` at configure time, so ninja
    // does not strictly need the augmented PATH; pass it anyway for robust rebuilds.
    run(Command::new("ninja")
        .env("PATH", &path)
        .arg("-C")
        .arg(&build_dir));

    // Link the freshly built static archive (`<build>/src/libdav1d.a`).
    println!(
        "cargo:rustc-link-search=native={}",
        build_dir.join("src").display()
    );
    println!("cargo:rustc-link-lib=static=dav1d");
    // libdav1d pulls in libm and pthread on Linux; on Apple platforms both live in
    // libSystem and need no explicit flag.
    if env("CARGO_CFG_TARGET_OS") != "macos" {
        println!("cargo:rustc-link-lib=dylib=m");
    }

    // Generate bindings from the vendored headers. `version.h` includes the
    // meson-generated `vcs_version.h`, so both include roots are required.
    let bindings = bindgen::Builder::default()
        .header(path_str(&manifest_dir.join("wrapper.h")))
        .clang_arg(format!("-I{}", path_str(&src.join("include"))))
        .clang_arg(format!("-I{}", path_str(&build_dir.join("include"))))
        .allowlist_function("dav1d_.*")
        .allowlist_type("Dav1d.*")
        .allowlist_var("DAV1D_.*")
        // Keep enum constants spelled exactly as in C (`DAV1D_PIXEL_LAYOUT_I444`)
        // rather than bindgen's default `Dav1dPixelLayout_DAV1D_PIXEL_LAYOUT_I444`.
        .prepend_enum_name(false)
        .layout_tests(false)
        .generate()
        .expect("generate dav1d FFI bindings");
    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("write dav1d bindings");

    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=build.rs");
    // Re-run when the vendored nasm tarball is swapped (e.g. a version bump).
    println!("cargo:rerun-if-changed={}", nasm_vendor::tarball_path());
    // Re-run when the submodule is bumped (its public version header moves).
    println!(
        "cargo:rerun-if-changed={}",
        path_str(&src.join("include/dav1d/version.h"))
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
