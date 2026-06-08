//! Build script: capture build-time provenance — git commit, working-tree state, build
//! profile, target triple, rustc version, and commit date — and expose them as compile-time
//! env vars so `gamut -V` can report a debuggable version string. Values default to `unknown`
//! when built outside a git checkout (e.g. from a crates.io tarball).

use std::process::Command;

fn main() {
    // Re-run when HEAD or the index changes so the commit hash and dirty flag stay current.
    // `--git-path` resolves these relative to the crate dir even though `.git` is at the
    // workspace root.
    for path in ["HEAD", "index"] {
        if let Some(resolved) = git(&["rev-parse", "--git-path", path]) {
            println!("cargo:rerun-if-changed={resolved}");
        }
    }

    let hash = git(&["rev-parse", "--short=12", "HEAD"]).unwrap_or_else(|| "unknown".into());
    // Any porcelain output means the working tree differs from HEAD (modified or untracked).
    let dirty = match git(&["status", "--porcelain"]) {
        Some(status) => !status.is_empty(),
        None => false,
    };
    let commit_date = git(&["log", "-1", "--format=%cI"]).unwrap_or_else(|| "unknown".into());

    let profile = env("PROFILE");
    let target = env("TARGET");
    let rustc_version = Command::new(std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into()))
        .arg("--version")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map_or_else(|| "unknown".into(), |s| s.trim().to_string());

    emit("GAMUT_GIT_HASH", &hash);
    emit("GAMUT_GIT_DIRTY", if dirty { "dirty" } else { "clean" });
    emit("GAMUT_COMMIT_DATE", &commit_date);
    emit("GAMUT_BUILD_PROFILE", &profile);
    emit("GAMUT_BUILD_TARGET", &target);
    emit("GAMUT_RUSTC_VERSION", &rustc_version);
}

/// Runs `git` with `args`, returning trimmed stdout on success or `None` on any failure
/// (git missing, not a repository, non-zero exit).
fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    Some(text.trim().to_string())
}

/// Reads a cargo-provided build env var, falling back to `unknown` if absent.
fn env(key: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| "unknown".into())
}

/// Emits a `key=value` pair as a compile-time env var for the crate being built.
fn emit(key: &str, value: &str) {
    println!("cargo:rustc-env={key}={value}");
}
