//! Builds a static `libaom` (cmake + ninja) from the vendored `third_party/aom`
//! submodule — the AV1 **reference codec** — and generates FFI bindings for its public
//! encode + decode API, plus a tiny C shim (`shim.c`) that unwraps libaom's
//! `aom_codec_{dec,enc}_init` macros.
//!
//! The build is hermetic: it never looks for a system-installed aom. The static archive,
//! the shim, and the bindings all land in `OUT_DIR`, so `cargo clean` fully resets them.

use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env("OUT_DIR"));
    let src = manifest_dir.join("../../third_party/aom");

    // Fail early with an actionable message if the submodule was not checked out.
    let cmakelists = src.join("CMakeLists.txt");
    assert!(
        cmakelists.exists(),
        "vendored aom not found at {} — `update = none` in .gitmodules keeps it out of a \
         recursive submodule update; run `mise run fetch-av1-oracles`",
        src.display()
    );

    // Build/locate a vendored nasm before configuring: libaom assembles its x86 SIMD with
    // nasm. `path` is the process PATH with the vendored nasm prepended (unchanged on
    // non-x86, where libaom uses intrinsics and needs no nasm).
    let path = path_with_nasm();

    let build_dir = out_dir.join("aom-build");
    // cmake's presence marker is `build.ninja`; on rebuilds we skip straight to the build.
    if !build_dir.join("build.ninja").exists() {
        run(Command::new("cmake")
            .env("PATH", &path)
            .arg("-S")
            .arg(&src)
            .arg("-B")
            .arg(&build_dir)
            .args(["-G", "Ninja"])
            .args([
                "-DCMAKE_BUILD_TYPE=Release",
                "-DBUILD_SHARED_LIBS=OFF",
                // The static archive links into a (position-independent) Rust test executable, so
                // its objects must be PIC — otherwise R_X86_64_32 relocations fail at link time.
                // (meson gives dav1d this by default; libaom's cmake needs it stated explicitly.)
                "-DCMAKE_POSITION_INDEPENDENT_CODE=ON",
                // Both directions on: the decoder validates gamut's encoder today; the
                // encoder is the reference bitstream source for the future gamut decoder.
                "-DCONFIG_AV1_ENCODER=1",
                "-DCONFIG_AV1_DECODER=1",
                // Trim everything not needed for the linkable static library.
                "-DENABLE_TESTS=0",
                "-DENABLE_EXAMPLES=0",
                "-DENABLE_TOOLS=0",
                "-DENABLE_DOCS=0",
            ]));
    }
    // Build only the static `aom` library target (its transitive deps come along).
    // `--parallel` carries an explicit count rather than being bare: a bare `--parallel` means
    // "no limit" to the build tool *and* overrides CMAKE_BUILD_PARALLEL_LEVEL, so it would
    // silently defeat the shared dial. (Stating the count also removes the question of what
    // this Ninja generator would have picked on its own — NCPUS + 2.)
    run(Command::new("cmake")
        .env("PATH", &path)
        .arg("--build")
        .arg(&build_dir)
        .arg("--parallel")
        .arg(build_env::build_parallelism().to_string())
        .args(["--target", "aom"]));

    // Compile the macro-unwrapping shim against libaom's public headers. `cc` emits its own
    // `rustc-link-lib=static=aomshim` *before* we emit the `aom` link line below, so the
    // linker sees aomshim (which references libaom) ahead of libaom and resolves cleanly.
    cc::Build::new()
        .file(manifest_dir.join("shim.c"))
        .include(&src)
        .compile("aomshim");

    // Link the freshly built static archive (`<build>/libaom.a`).
    println!("cargo:rustc-link-search=native={}", path_str(&build_dir));
    println!("cargo:rustc-link-lib=static=aom");
    // libaom pulls in libm and pthread on Linux; on Apple platforms both live in libSystem.
    if env("CARGO_CFG_TARGET_OS") != "macos" {
        println!("cargo:rustc-link-lib=dylib=m");
        println!("cargo:rustc-link-lib=dylib=pthread");
    }

    // Generate bindings from the vendored public headers (self-contained; no build-tree
    // config header is required for the API surface we use).
    let bindings = bindgen::Builder::default()
        .header(path_str(&manifest_dir.join("wrapper.h")))
        .clang_arg(format!("-I{}", path_str(&src)))
        .allowlist_function("aom_.*")
        .allowlist_function("aomshim_.*")
        // `aom.*` (no underscore) so the encoder-control enum `aome_enc_control_id` — and its
        // `AOME_*`/`AV1E_*` variant constants — come along, not just the `aom_*` types.
        .allowlist_type("aom.*")
        .allowlist_var("AOM_.*")
        // Keep enum constants spelled exactly as in C (`AOM_IMG_FMT_I444`, `AOM_CODEC_OK`)
        // rather than bindgen's default type-prefixed form.
        .prepend_enum_name(false)
        .layout_tests(false)
        .generate()
        .expect("generate aom FFI bindings");
    bindings
        .write_to_file(out_dir.join("bindings.rs"))
        .expect("write aom bindings");

    println!("cargo:rerun-if-changed=wrapper.h");
    println!("cargo:rerun-if-changed=shim.c");
    println!("cargo:rerun-if-changed=build.rs");
    // Re-run when the vendored nasm tarball is swapped (e.g. a version bump).
    println!("cargo:rerun-if-changed={}", nasm_vendor::tarball_path());
    // Re-run when the submodule is bumped (its public ABI header moves).
    println!(
        "cargo:rerun-if-changed={}",
        path_str(&src.join("aom/aom_codec.h"))
    );
}

/// The process `PATH` with the vendored nasm directory prepended, so cmake's assembler
/// search resolves to it. On arches/platforms where no vendored nasm is built (non-x86,
/// non-Unix), returns the unchanged `PATH`.
fn path_with_nasm() -> std::ffi::OsString {
    // Base on the launcher-shim-filtered PATH, not the raw one: `run` applies the same
    // filtering but deliberately will not overwrite a PATH the command already sets, so the
    // filtering has to happen here for these commands.
    let base = build_env::BuildEnv::detect().path();
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
    // Normalise any compiler-launcher env (sccache/ccache) into the single position cmake
    // defines for it, so this build is hermetic to exactly what it configures. See the
    // `build-env` crate docs; `GAMUT_BUILD_KEEP_ENV=1` opts out.
    build_env::sanitize(cmd);
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn {cmd:?}: {e}"));
    assert!(status.success(), "command failed ({status}): {cmd:?}");
}
