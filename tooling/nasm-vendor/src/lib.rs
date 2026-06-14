//! Dev-only helper that builds a vendored [`nasm`](https://www.nasm.us) assembler from a
//! committed release source tarball, so the dav1d / libavif oracle build scripts can
//! assemble dav1d's x86 SIMD without a system-installed nasm.
//!
//! Why a *release tarball* rather than a git submodule: a release tarball ships a
//! pre-generated `configure` and pre-generated C sources, so it builds with only a C
//! compiler + `make` + `sh` — tools already required to build the C oracles, on every
//! platform. A git checkout would instead need `autoconf`/`automake`/`m4`/`perl`.
//!
//! The build is hermetic and cached in the caller's `OUT_DIR`, so `cargo clean` fully
//! resets it. It is also a no-op on targets that do not need nasm (see [`ensure_nasm`]).

use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

/// nasm version we vendor. Bump together with `vendor/nasm-<VERSION>.tar.xz` and
/// [`TARBALL_SHA256`].
const NASM_VERSION: &str = "2.16.03";

/// SHA-256 of the committed `vendor/nasm-2.16.03.tar.xz`.
///
/// This is the exact upstream artifact from
/// <https://www.nasm.us/pub/nasm/releasebuilds/2.16.03/nasm-2.16.03.tar.xz>
/// (upstream MD5 `2b8c72c52eee4f20085065e68ac83b55`). Verified before every build so a
/// tampered or corrupt source tree can never be compiled.
const TARBALL_SHA256: &str = "1412a1c760bbd05db026b6c0d1657affd6631cd0a63cddb6f73cc6d4aa616148";

/// Absolute path to this crate's committed tarball. `env!` expands at *this* crate's
/// compile time, so it resolves to `tooling/nasm-vendor/vendor/...` regardless of which
/// host crate later calls into us.
const TARBALL: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/vendor/nasm-2.16.03.tar.xz");

/// Absolute path of the committed nasm tarball, for callers' `cargo:rerun-if-changed`.
#[must_use]
pub fn tarball_path() -> &'static str {
    TARBALL
}

/// Ensures a `nasm` binary exists, building it from the vendored source if needed.
///
/// Returns `Some(dir)` where `dir` contains the freshly built `nasm` executable — prepend
/// it to `PATH` so meson's `find_program('nasm')` resolves to it. Returns `None` when no
/// vendored nasm is needed or buildable, leaving the caller's `PATH` untouched (any system
/// nasm is then used):
///   * non-x86 target arch — dav1d assembles its `.S` files with the C compiler there, so
///     nasm is never invoked;
///   * non-Unix host — we do not vendor a Windows build (no Windows CI); fall back to a
///     system nasm if present.
///
/// The result is cached in the caller's `OUT_DIR`: a second call returns the already-built
/// binary without re-running `configure`/`make`, and `cargo clean` resets it.
///
/// # Panics
/// On x86 Unix, panics (aborting the build, as a build script should) if the tarball is
/// missing, fails SHA-256 verification, extracts to an unexpected layout, or the
/// `configure`/`make` build fails.
#[must_use]
pub fn ensure_nasm() -> Option<PathBuf> {
    // dav1d only calls `find_program('nasm')` for the x86 asm; on other arches there is
    // nothing to assemble with nasm, so skip the whole build.
    let arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    if arch != "x86" && arch != "x86_64" {
        return None;
    }
    // We only know how to build nasm via `sh configure && make`. Skip on Windows.
    if cfg!(windows) {
        return None;
    }

    let out_dir = PathBuf::from(env("OUT_DIR"));
    let work = out_dir.join("nasm-vendor");
    let src_dir = work.join(format!("nasm-{NASM_VERSION}"));
    // A release-tarball `make` drops the binary as `nasm` at the source root.
    let nasm_bin = src_dir.join("nasm");

    // Idempotency: mirror the oracles' `build.ninja`/`CMakeCache.txt` skip checks.
    if nasm_bin.is_file() {
        return Some(src_dir);
    }

    let bytes = std::fs::read(TARBALL).unwrap_or_else(|e| {
        panic!(
            "vendored nasm tarball not found at {TARBALL}: {e} \
             (was it committed under tooling/nasm-vendor/vendor/?)"
        )
    });
    verify_sha256(&bytes);

    // Extract fresh — clear any tree left by an interrupted previous build.
    if work.exists() {
        std::fs::remove_dir_all(&work).expect("clear stale nasm-vendor work dir");
    }
    std::fs::create_dir_all(&work).expect("create nasm-vendor work dir");
    extract_tar_xz(&bytes, &work);
    assert!(
        src_dir.join("configure").is_file(),
        "extracted nasm tree missing `configure` at {} — unexpected tarball layout",
        src_dir.display()
    );

    // A *release* tarball ships a pre-generated `configure` + pre-generated C sources, so
    // only cc + make + sh are needed (no autoconf/automake/perl). `make nasm` builds just
    // the assembler, skipping ndisasm/manpages.
    run(Command::new("sh").arg("configure").current_dir(&src_dir));
    let jobs = std::env::var("NUM_JOBS").unwrap_or_else(|_| "4".to_string());
    run(Command::new("make")
        .arg(format!("-j{jobs}"))
        .arg("nasm")
        .current_dir(&src_dir));

    assert!(
        nasm_bin.is_file(),
        "nasm build completed but binary not found at {}",
        nasm_bin.display()
    );
    Some(src_dir)
}

/// Aborts the build unless `bytes` hashes to [`TARBALL_SHA256`].
fn verify_sha256(bytes: &[u8]) {
    let got = hex_lower(&Sha256::digest(bytes));
    assert!(
        got == TARBALL_SHA256,
        "nasm tarball SHA-256 mismatch:\n  expected {TARBALL_SHA256}\n  got      {got}\n\
         refusing to build a tampered or corrupt nasm source tree"
    );
}

/// Decodes the `.tar.xz` (pure-Rust xz via `lzma-rs`) and unpacks it into `dest`.
fn extract_tar_xz(bytes: &[u8], dest: &Path) {
    let mut tar_bytes = Vec::new();
    lzma_rs::xz_decompress(&mut std::io::Cursor::new(bytes), &mut tar_bytes)
        .expect("xz-decompress nasm tarball");
    tar::Archive::new(std::io::Cursor::new(tar_bytes))
        .unpack(dest)
        .expect("unpack nasm tarball");
}

/// Lowercase hex encoding, avoiding a dependency on the `hex` crate.
fn hex_lower(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Reads a required build-time env var, panicking (this runs inside a build script) if absent.
fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| panic!("missing build env var {key}"))
}

/// Runs a build subcommand, aborting the build with its output on failure.
/// (Mirrors the `run` helper in the oracle build scripts.)
fn run(cmd: &mut Command) {
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("failed to spawn {cmd:?}: {e}"));
    assert!(status.success(), "command failed ({status}): {cmd:?}");
}
