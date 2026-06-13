//! Builds a static `libpng` (against a static `zlib`) from the `third_party/` submodules and
//! generates FFI bindings for libpng's public API. Everything lands under `OUT_DIR`, so the shipped
//! gamut crates never need a C toolchain — this oracle is pulled in only as a dev-dependency.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env("OUT_DIR"));
    let zlib_submodule = manifest_dir.join("../../third_party/zlib");
    let png_src = manifest_dir.join("../../third_party/libpng");

    for (name, src) in [("zlib", &zlib_submodule), ("libpng", &png_src)] {
        assert!(
            src.join("CMakeLists.txt").exists(),
            "vendored {name} not found under {} — run `git submodule update --init --recursive`",
            src.display()
        );
    }
    // zlib's CMake deletes the in-source `zconf.h` during an out-of-source build; build from a
    // private copy under OUT_DIR so the vendored submodule stays pristine.
    let zlib_src = out_dir.join("zlib-src");
    if !zlib_src.join("CMakeLists.txt").exists() {
        copy_dir(&zlib_submodule, &zlib_src);
    }

    // ---- 1. Build + install a static, PIC zlib so libpng can find it and we can link it. -------
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

    // ---- 2. Build a static, PIC libpng against that zlib. -------------------------------------
    let png_build = out_dir.join("libpng-build");
    if !png_build.join("CMakeCache.txt").exists() {
        run(Command::new("cmake")
            .arg("-S")
            .arg(&png_src)
            .arg("-B")
            .arg(&png_build)
            .args([
                "-DCMAKE_BUILD_TYPE=Release",
                "-DCMAKE_POSITION_INDEPENDENT_CODE=ON",
                "-DPNG_SHARED=OFF",
                "-DPNG_STATIC=ON",
                "-DPNG_TESTS=OFF",
                "-DPNG_TOOLS=OFF",
                "-DPNG_FRAMEWORK=OFF",
                &format!("-DZLIB_ROOT={}", path_str(&zlib_prefix)),
                &format!("-DCMAKE_PREFIX_PATH={}", path_str(&zlib_prefix)),
            ]));
    }
    run(Command::new("cmake").arg("--build").arg(&png_build).args([
        "--config",
        "Release",
        "--parallel",
    ]));

    // ---- 3. Link the static libpng (name varies: libpng16.a / libpng.a) + zlib + libm. --------
    let png_lib = find_static_lib(&png_build, "png").expect("static libpng archive not found");
    let link_dir = png_lib
        .parent()
        .expect("libpng archive has a parent dir")
        .to_path_buf();
    println!("cargo:rustc-link-search=native={}", path_str(&link_dir));
    println!(
        "cargo:rustc-link-search=native={}",
        path_str(&zlib_prefix.join("lib"))
    );
    println!("cargo:rustc-link-lib=static={}", lib_link_name(&png_lib));
    println!("cargo:rustc-link-lib=static=z");
    if env("CARGO_CFG_TARGET_OS") != "macos" {
        println!("cargo:rustc-link-lib=dylib=m");
    }

    // ---- 4. Bindings. png.h needs pnglibconf.h (build tree) and zlib.h (install prefix). ------
    let bindings = bindgen::Builder::default()
        .header(path_str(&manifest_dir.join("wrapper.h")))
        .clang_arg(format!("-I{}", path_str(&png_src)))
        .clang_arg(format!("-I{}", path_str(&png_build)))
        .clang_arg(format!("-I{}", path_str(&zlib_prefix.join("include"))))
        .allowlist_function("png_.*")
        .allowlist_type("png_.*")
        .allowlist_var("PNG_.*")
        .prepend_enum_name(false)
        .layout_tests(false)
        .generate()
        .expect("generate libpng FFI bindings");
    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("write libpng bindings");

    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", path_str(&png_src.join("png.h")));
}

/// Recursively copies `src` into `dst` (skipping `.git`) so an in-source-modifying build never
/// touches the vendored submodule.
fn copy_dir(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap_or_else(|e| panic!("create {}: {e}", dst.display()));
    for entry in fs::read_dir(src)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", src.display()))
        .flatten()
    {
        let path = entry.path();
        if path.file_name().is_some_and(|n| n == ".git") {
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

/// Finds a static archive `lib<...stem...>.a` directly inside `dir`.
fn find_static_lib(dir: &Path, stem: &str) -> Option<PathBuf> {
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if let Some(name) = path.file_name().and_then(|s| s.to_str())
            && name.starts_with("lib")
            && name.contains(stem)
            && name.ends_with(".a")
        {
            return Some(path);
        }
    }
    None
}

/// `libpng16.a` → `png16` (the `-l` link name).
fn lib_link_name(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .expect("archive file stem");
    stem.strip_prefix("lib").unwrap_or(stem).to_string()
}

fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("missing build env var {key}"))
}

fn path_str(p: &Path) -> String {
    p.to_str()
        .unwrap_or_else(|| panic!("non-UTF-8 path: {}", p.display()))
        .to_string()
}

fn run(cmd: &mut Command) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn {cmd:?}: {e}"));
    assert!(status.success(), "command failed ({status}): {cmd:?}");
}
