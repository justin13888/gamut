//! Builds a static Little-CMS (lcms2) from the `third_party/lcms2` submodule and generates FFI
//! bindings for its public API.
//!
//! lcms2 is dependency-free C99 — it pulls in no zlib/jpeg/system libraries — so the whole archive
//! is compiled straight from `src/*.c` with the `cc` crate (no CMake). Everything lands under
//! `OUT_DIR`, so `cargo clean` fully resets the build. CI needs the submodule checked out
//! (`git submodule update --init --recursive`) and a C toolchain plus libclang (for bindgen);
//! nothing else.

use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(env("CARGO_MANIFEST_DIR"));
    let out = PathBuf::from(env("OUT_DIR"));
    let src = manifest.join("../../third_party/lcms2");
    let csrc = src.join("src");
    let include = src.join("include");

    assert!(
        csrc.join("cmsxform.c").exists(),
        "vendored lcms2 not found under {} — run `git submodule update --init --recursive`",
        src.display()
    );

    // ---- Compile the whole lcms2 source set into one static archive. ---------------------------
    let mut build = cc::Build::new();
    build.include(&include).include(&csrc);
    // Vendored reference code we do not own: silence its warnings rather than treat them as signal.
    build.flag_if_supported("-w");
    let mut count = 0;
    for entry in std::fs::read_dir(&csrc).expect("read lcms2 src dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) == Some("c") {
            build.file(&path);
            count += 1;
        }
    }
    assert!(
        count > 20,
        "expected the full lcms2 source set, found only {count} files"
    );
    // Build parallelism: `cc::Build` spawns NUM_JOBS compilers — the whole lcms2 source set at
    // once — and cargo derives NUM_JOBS from `--jobs`/`CARGO_BUILD_JOBS`. That is the same value
    // `tooling/build-env`'s `build_parallelism` falls back to, so one `CARGO_BUILD_JOBS` bounds
    // this compile and the cmake/ninja oracles together; there is deliberately no dial here.
    build.compile("lcms2");

    // lcms2 uses libm (`pow`/`floor`/…); link it on platforms that split it from libc.
    if env("CARGO_CFG_TARGET_OS") != "macos" {
        println!("cargo:rustc-link-lib=dylib=m");
    }

    // ---- Bindings. -----------------------------------------------------------------------------
    let bindings = bindgen::Builder::default()
        .header(path_str(&manifest.join("wrapper.h")))
        .clang_arg(format!("-I{}", path_str(&include)))
        .allowlist_function("cms.*")
        .allowlist_type("cms.*")
        .allowlist_var("INTENT_.*")
        // lcms2's signature enums carry values up to 0x7FFFFFFF; emit them as plain consts + a
        // primitive type alias so the FFI args are simple integers, not generated Rust enums.
        .default_enum_style(bindgen::EnumVariation::Consts)
        .prepend_enum_name(false)
        .layout_tests(false)
        .generate()
        .expect("generate lcms2 FFI bindings");
    bindings
        .write_to_file(out.join("bindings.rs"))
        .expect("write lcms2 bindings");

    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=build.rs");
    println!(
        "cargo:rerun-if-changed={}",
        path_str(&include.join("lcms2.h"))
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
