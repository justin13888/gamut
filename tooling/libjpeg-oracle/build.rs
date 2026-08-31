//! Builds a static `libjpeg-turbo` from the `third_party/libjpeg-turbo` submodule (tag 3.2.0) and
//! compiles the `src/shim.c` bridge against it. Everything lands under `OUT_DIR`, so the shipped
//! gamut crates never need a C toolchain — this oracle is pulled in only as a dev-dependency.
//!
//! No bindgen: libjpeg's error model is `setjmp`/`longjmp`, which cannot be driven from Rust, so the
//! whole encode/decode surface lives in `src/shim.c` behind a small `extern "C"` API. Rust hand-
//! declares those few functions (as the exiv2/iptc/dng shim oracles do), which is leaner than
//! generating the entire `jpeg_*` header and then reaching around it for the error handling.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env("OUT_DIR"));
    let src = manifest_dir.join("../../third_party/libjpeg-turbo");

    assert!(
        src.join("CMakeLists.txt").exists(),
        "vendored libjpeg-turbo not found under {} — run `git submodule update --init --recursive`",
        src.display()
    );

    // ---- CMake-build a static, PIC libjpeg-turbo. ------------------------------------------
    let build = out_dir.join("libjpeg-build");
    if !build.join("CMakeCache.txt").exists() {
        run(Command::new("cmake")
            .arg("-S")
            .arg(&src)
            .arg("-B")
            .arg(&build)
            .args([
                "-DCMAKE_BUILD_TYPE=Release",
                // Static archive only, no shared object; we link it into the Rust test binary.
                "-DENABLE_SHARED=0",
                "-DENABLE_STATIC=1",
                // WITH_SIMD=0 drops the x86 SIMD kernels and, with them, the nasm build dependency.
                // The oracle is a correctness reference, not a benchmark, so the C scalar paths are
                // fine — its speed is irrelevant and this keeps the build hermetic (no assembler).
                "-DWITH_SIMD=0",
                // We only need the classic libjpeg API (jpeg_mem_src/dest); skip the TurboJPEG wrapper.
                "-DWITH_TURBOJPEG=0",
                // The static archive links into a (position-independent) Rust test executable, so its
                // objects must be PIC — otherwise R_X86_64_32 relocations fail at link time on
                // PIE-by-default toolchains (e.g. Fedora).
                "-DCMAKE_POSITION_INDEPENDENT_CODE=ON",
            ]));
    }
    // `--parallel` takes an explicit count. Bare, it becomes `make -j` with no limit *and*
    // overrides CMAKE_BUILD_PARALLEL_LEVEL, so the shared dial would be silently ignored.
    run(Command::new("cmake").arg("--build").arg(&build).args([
        "--config",
        "Release",
        "--parallel",
        &build_env::build_parallelism().to_string(),
    ]));

    // ---- Compile the shim against the freshly built headers. -------------------------------
    // jpeglib.h lives in the submodule's `src/`; the generated `jconfig.h` (with
    // LIBJPEG_TURBO_VERSION / MEM_SRCDST_SUPPORTED) is written to the build-tree root.
    cc::Build::new()
        .file(manifest_dir.join("src/shim.c"))
        .include(src.join("src"))
        .include(&build)
        .flag_if_supported("-w")
        .compile("libjpeg_oracle_shim");

    // ---- Link the static libjpeg after the shim so the shim's undefined jpeg_* symbols resolve. --
    let jpeg_lib = find_static_lib(&build, "jpeg").expect("static libjpeg archive not found");
    let link_dir = jpeg_lib
        .parent()
        .expect("libjpeg archive has a parent dir")
        .to_path_buf();
    println!("cargo:rustc-link-search=native={}", path_str(&link_dir));
    println!("cargo:rustc-link-lib=static=jpeg");
    if env("CARGO_CFG_TARGET_OS") != "macos" {
        println!("cargo:rustc-link-lib=dylib=m");
    }

    println!("cargo:rerun-if-changed=src/shim.c");
    println!("cargo:rerun-if-changed=build.rs");
    println!(
        "cargo:rerun-if-changed={}",
        path_str(&src.join("src/jpeglib.h"))
    );
}

/// Recursively finds a static archive `lib<stem>.a` under `dir` (libjpeg-turbo emits `libjpeg.a`
/// in the build-tree root, but the exact location varies by CMake generator).
fn find_static_lib(dir: &Path, stem: &str) -> Option<PathBuf> {
    let target = format!("lib{stem}.a");
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_static_lib(&path, stem) {
                return Some(found);
            }
        } else if path.file_name().and_then(|s| s.to_str()) == Some(target.as_str()) {
            return Some(path);
        }
    }
    None
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
