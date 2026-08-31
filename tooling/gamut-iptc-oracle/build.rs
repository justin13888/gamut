//! Builds a static `libexiv2` from the `third_party/exiv2` submodule (XMP/PNG and every optional
//! feature off, so the build pulls in no Expat, zlib, or network dependency) and links it behind the
//! `extern "C"` shim in `src/oracle_shim.cpp`.
//!
//! Everything lands under `OUT_DIR`, so `cargo clean` fully resets the build. CI needs the submodule
//! checked out (`git submodule update --init --recursive`) and a C++ toolchain + CMake/Ninja (from
//! mise); nothing else.

use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(env("CARGO_MANIFEST_DIR"));
    let out = PathBuf::from(env("OUT_DIR"));
    let src = manifest.join("../../third_party/exiv2");

    assert!(
        src.join("CMakeLists.txt").exists(),
        "vendored exiv2 not found under {} — run `git submodule update --init --recursive`",
        src.display()
    );

    // ---- CMake-build a static libexiv2 with XMP/PNG and all optional features disabled. ----------
    let build = out.join("exiv2-build");
    if !build.join("CMakeCache.txt").exists() {
        run(Command::new("cmake")
            .arg("-S")
            .arg(&src)
            .arg("-B")
            .arg(&build)
            .args([
                "-GNinja",
                "-DCMAKE_BUILD_TYPE=Release",
                "-DBUILD_SHARED_LIBS=OFF",
                // PIC so the static archive links into Rust test binaries on PIE-by-default toolchains.
                "-DCMAKE_POSITION_INDEPENDENT_CODE=ON",
                // XMP off → no Expat; PNG off → no zlib; INIH off → no FetchContent (hermetic).
                "-DEXIV2_ENABLE_XMP=OFF",
                "-DEXIV2_ENABLE_PNG=OFF",
                "-DEXIV2_ENABLE_NLS=OFF",
                "-DEXIV2_ENABLE_CURL=OFF",
                "-DEXIV2_ENABLE_WEBREADY=OFF",
                "-DEXIV2_ENABLE_BMFF=OFF",
                "-DEXIV2_ENABLE_INIH=OFF",
                "-DEXIV2_BUILD_SAMPLES=OFF",
                "-DEXIV2_BUILD_EXIV2_COMMAND=OFF",
                "-DEXIV2_BUILD_UNIT_TESTS=OFF",
                "-DEXIV2_BUILD_DOC=OFF",
            ]));
    }
    // `--parallel` with an explicit count, so this C++ exiv2 tree is bounded by the shared dial
    // rather than by whatever the default generator would pick.
    run(Command::new("cmake")
        .arg("--build")
        .arg(&build)
        .arg("--parallel")
        .arg(build_env::build_parallelism().to_string())
        .args(["--config", "Release"]));

    let lib_dir = build.join("lib");
    assert!(
        lib_dir.join("libexiv2.a").exists(),
        "exiv2 static library not produced under {}",
        lib_dir.display()
    );

    // ---- Compile the shim (cc links the C++ runtime) and link the exiv2 archive. -----------------
    let mut shim = cc::Build::new();
    shim.cpp(true).std("c++17");
    shim.include(src.join("include"));
    shim.include(&build); // generated exv_conf.h
    shim.flag_if_supported("-w");
    shim.file(manifest.join("src/oracle_shim.cpp"));
    shim.compile("gamut_iptc_oracle_shim");

    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    println!("cargo:rustc-link-lib=static=exiv2");

    println!("cargo:rerun-if-changed=src/oracle_shim.cpp");
    println!("cargo:rerun-if-changed=build.rs");
}

/// Runs a build-step command, panicking (this is a build script) if it fails.
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

/// Reads a required build-time env var, panicking if absent.
fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("missing build env var {key}"))
}
