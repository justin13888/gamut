//! Statically builds the reference libjxl v0.12.0 (via the `jpegxl-src` crate) and emits the link
//! directives that let downstream crates call the FFI declarations in `src/`.
//!
//! `jpegxl-src` vendors the complete libjxl source (highway/brotli/skcms) inside its published
//! package and drives a hermetic cmake build entirely under `OUT_DIR`, so `cargo clean` fully resets
//! it and no git submodules or system-installed libjxl are ever consulted. The build emits the
//! `rustc-link-lib` lines for `jxl`, `jxl_cms`, `jxl_threads`, `hwy` and the brotli libraries, plus
//! the platform C++ runtime.
//!
//! Three special target situations:
//!
//! - `GAMUT_JXL_SYS_SKIP_NATIVE=1` skips the native build entirely (see below).
//! - `wasm32-unknown-emscripten` builds the **same vendored tree** with the emsdk toolchain
//!   ([`build_emscripten`]) — `jpegxl_src::build()` cannot be reused there because it emits a
//!   host-side `stdc++`/`c++` link line and cannot disable libjxl's wasm pthread flags.
//! - Any other `wasm32` target (`wasm32-unknown-unknown`, `wasm32-wasip*`) skips the native build
//!   unconditionally: no C/C++ toolchain can produce archives linkable into those targets, and
//!   `gamut-jxl` compiles its encoder out there anyway, so nothing ever references the symbols.

/// Must match the exact `jpegxl-src = "=0.12.0"` pin in Cargo.toml: the emscripten lane locates
/// jpegxl-src's vendored libjxl tree by this version string. If the pin is bumped without this
/// constant, [`vendored_libjxl_dir`] panics loudly — bump both together (and re-verify the FFI
/// declarations plus the cmake define mirror in [`build_emscripten`]).
const JPEGXL_SRC_VERSION: &str = "0.12.0";

fn main() {
    // Escape hatch for `cargo check`-only environments (the extended `check-cross`/`check-msrv` CI
    // boxes) that lack cmake or a cross C++ toolchain. This is safe because `cargo check` compiles
    // but never *links*, so the absent native library is never referenced. When set, we skip the
    // native build and emit no link lines — anything that actually links (tests, binaries) must
    // NOT set it. The build re-runs if either variable's value changes.
    println!("cargo:rerun-if-env-changed=GAMUT_JXL_SYS_SKIP_NATIVE");
    println!("cargo:rerun-if-env-changed=GAMUT_JXL_SYS_LIBJXL_DIR");
    println!("cargo:rerun-if-env-changed=CMAKE_BUILD_PARALLEL_LEVEL");
    if std::env::var("GAMUT_JXL_SYS_SKIP_NATIVE").as_deref() == Ok("1") {
        println!(
            "cargo:warning=GAMUT_JXL_SYS_SKIP_NATIVE=1: skipping the libjxl static build; \
             gamut-jxl-sys will not link (check-only)."
        );
        return;
    }

    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
    match (arch.as_str(), os.as_str()) {
        // wasm without emscripten: no C/C++ toolchain targets this ABI, so a native libjxl can
        // never link here. Skip unconditionally (no env var needed) — gamut-jxl gates the encoder
        // out on these targets, so the symbols are never referenced.
        ("wasm32", other) if other != "emscripten" => {
            println!(
                "cargo:warning=gamut-jxl-sys: no native libjxl for wasm32-{other}; emitting no \
                 link lines (gamut-jxl compiles its encoder out on this target)."
            );
        }
        // emscripten: build the same vendored libjxl with the emsdk toolchain.
        ("wasm32", "emscripten") => {
            build_emscripten();
            emit_include_metadata();
        }
        // Everything else: the stock native build.
        _ => {
            jpegxl_src::build();
            emit_include_metadata();
        }
    }
}

/// Publishes the installed libjxl headers to dependent build scripts as `DEP_JXL_INCLUDE`
/// (`links = "jxl"` + `cargo:include=`), so a crate compiling C/C++ against this exact libjxl —
/// e.g. the dev-only Adobe DNG SDK oracle — uses matching headers rather than a vendored copy.
///
/// The cmake install prefix is this build script's `OUT_DIR` (both `jpegxl_src::build()` and
/// [`build_emscripten`] install there), so the headers land under `OUT_DIR/include`.
fn emit_include_metadata() {
    let include = std::path::PathBuf::from(env("OUT_DIR")).join("include");
    assert!(
        include.join("jxl").join("decode.h").exists(),
        "libjxl build did not install headers under {}",
        include.display()
    );
    println!("cargo:include={}", include.display());
}

/// Reads a required build-time env var, panicking (this is a build script) if absent.
fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("missing build env var {key}"))
}

/// Builds the vendored libjxl for `wasm32-unknown-emscripten` and emits its link directives.
///
/// Mirrors `jpegxl_src::build()`'s cmake define set (keep the two in sync on pin bumps), with two
/// wasm-specific differences:
///
/// - `JPEGXL_ENABLE_WASM_THREADS=OFF`: gamut-jxl never uses the parallel runner (`jxl_threads` is
///   not even linked), and Rust's emscripten target links without `-pthread`, which would reject
///   pthread-flavoured (shared-memory/atomics) objects.
/// - No C++-runtime link line: rustc drives the final link through `emcc`, which provides libc++
///   itself (the native path's `stdc++`/`c++` line would fail here).
///
/// The `cmake` crate wraps configure/build in `emcmake`/`emmake` automatically for `*-emscripten`
/// targets; emsdk (`emcc` on `PATH`) must be installed and activated.
fn build_emscripten() {
    let source = vendored_libjxl_dir();
    let mut config = cmake::Config::new(&source);
    config
        .define("BUILD_TESTING", "OFF")
        .define("BUILD_SHARED_LIBS", "OFF")
        .define("JPEGXL_ENABLE_TOOLS", "OFF")
        .define("JPEGXL_ENABLE_DOXYGEN", "OFF")
        .define("JPEGXL_ENABLE_MANPAGES", "OFF")
        .define("JPEGXL_ENABLE_BENCHMARK", "OFF")
        .define("JPEGXL_ENABLE_EXAMPLES", "OFF")
        .define("JPEGXL_ENABLE_JNI", "OFF")
        .define("JPEGXL_ENABLE_SJPEG", "OFF")
        .define("JPEGXL_ENABLE_OPENEXR", "OFF")
        .define("JPEGXL_BUNDLE_LIBPNG", "OFF")
        .define("JPEGXL_ENABLE_WASM_THREADS", "OFF");
    // Build parallelism: honour an operator-set `CMAKE_BUILD_PARALLEL_LEVEL`, and only fall back
    // to the CPU count when nothing was asked for. `available_parallelism` counts cores, not
    // memory, and libjxl's C++ translation units are among the most memory-hungry things this
    // workspace compiles — so on a many-core machine under a memory cap (a container, a systemd
    // slice, a mutation-testing run with several build scenarios in flight) the core count is far
    // too many compilers at once. Setting the variable unconditionally left the operator no way to
    // say so; every other vendored build here takes `cmake --build --parallel`, which reads the
    // same variable, so this is now the one dial for all of them.
    if std::env::var_os("CMAKE_BUILD_PARALLEL_LEVEL").is_none()
        && let Ok(parallelism) = std::thread::available_parallelism()
    {
        config.env("CMAKE_BUILD_PARALLEL_LEVEL", parallelism.to_string());
    }
    let prefix = config.build();

    let lib_dir = ["lib", "lib64"]
        .iter()
        .map(|dir| prefix.join(dir))
        .find(|dir| dir.exists())
        .unwrap_or_else(|| panic!("no lib/ or lib64/ under {}", prefix.display()));
    println!("cargo:rustc-link-search=native={}", lib_dir.display());
    for lib in [
        "jxl",
        "jxl_cms",
        "hwy",
        "brotlidec",
        "brotlienc",
        "brotlicommon",
    ] {
        println!("cargo:rustc-link-lib=static={lib}");
    }
}

/// Locates jpegxl-src's vendored libjxl source tree.
///
/// `GAMUT_JXL_SYS_LIBJXL_DIR` overrides the search (for `cargo vendor` layouts and source
/// mirrors); otherwise the extracted jpegxl-src package is found under Cargo's registry sources.
/// `jpegxl_src::source_dir()` is private upstream, so the path cannot simply be asked for —
/// tracked for upstreaming.
fn vendored_libjxl_dir() -> std::path::PathBuf {
    use std::path::PathBuf;

    if let Some(dir) = std::env::var_os("GAMUT_JXL_SYS_LIBJXL_DIR") {
        let dir = PathBuf::from(dir);
        assert!(
            dir.is_dir(),
            "GAMUT_JXL_SYS_LIBJXL_DIR is not a directory: {}",
            dir.display()
        );
        return dir;
    }

    #[allow(deprecated)] // `home_dir` is un-deprecated since Rust 1.87; the lint name lags.
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::home_dir().map(|home| home.join(".cargo")))
        .expect("neither CARGO_HOME nor a home directory is available");
    let src_root = cargo_home.join("registry").join("src");
    let needle = format!("jpegxl-src-{JPEGXL_SRC_VERSION}");
    if let Ok(entries) = std::fs::read_dir(&src_root) {
        for index_dir in entries.flatten() {
            let candidate = index_dir.path().join(&needle).join("libjxl");
            if candidate.is_dir() {
                return candidate;
            }
        }
    }
    panic!(
        "could not locate the vendored libjxl of {needle} under {}; set GAMUT_JXL_SYS_LIBJXL_DIR \
         to a libjxl source tree (e.g. for cargo-vendor setups), and if the jpegxl-src pin was \
         bumped update JPEGXL_SRC_VERSION in build.rs to match",
        src_root.display()
    );
}
