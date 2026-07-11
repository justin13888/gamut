//! Statically builds the reference libjxl v0.12.0 (via the `jpegxl-src` crate) and emits the link
//! directives that let downstream crates call the FFI declarations in `src/`.
//!
//! `jpegxl-src` vendors the complete libjxl source (highway/brotli/skcms) inside its published
//! package and drives a hermetic cmake build entirely under `OUT_DIR`, so `cargo clean` fully resets
//! it and no git submodules or system-installed libjxl are ever consulted. The build emits the
//! `rustc-link-lib` lines for `jxl`, `jxl_cms`, `jxl_threads`, `hwy` and the brotli libraries, plus
//! the platform C++ runtime.
//!
//! Set `GAMUT_JXL_SYS_SKIP_NATIVE=1` to skip the native build entirely (see below).

fn main() {
    // Escape hatch for `cargo check`-only environments (the extended `check-cross`/`check-msrv` CI
    // boxes) that lack cmake or a cross C++ toolchain. This is safe because `cargo check` compiles
    // but never *links*, so the absent native library is never referenced. When set, we skip
    // `jpegxl_src::build()` and emit no link lines — anything that actually links (tests, binaries)
    // must NOT set it. The build re-runs if the variable's value changes.
    println!("cargo:rerun-if-env-changed=GAMUT_JXL_SYS_SKIP_NATIVE");
    if std::env::var("GAMUT_JXL_SYS_SKIP_NATIVE").as_deref() == Ok("1") {
        println!(
            "cargo:warning=GAMUT_JXL_SYS_SKIP_NATIVE=1: skipping the libjxl static build; \
             gamut-jxl-sys will not link (check-only)."
        );
        return;
    }

    // Configure + build libjxl v0.12.0 statically under OUT_DIR and emit its link directives.
    jpegxl_src::build();
}
