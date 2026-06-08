//! Build script: capture build-time provenance — git commit, working-tree state, build
//! profile, target triple, rustc version, commit date, and build timestamp — and expose them
//! as compile-time env vars so `gamut -V` can report a debuggable version string. Values
//! default to `unknown` when built outside a git checkout (e.g. from a crates.io tarball).

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    // Re-run when HEAD or the index changes so the commit hash and dirty flag stay current.
    // `--git-path` resolves these relative to the crate dir even though `.git` is at the
    // workspace root.
    for path in ["HEAD", "index"] {
        if let Some(resolved) = git(&["rev-parse", "--git-path", path]) {
            println!("cargo:rerun-if-changed={resolved}");
        }
    }
    // Reproducible builds: honor SOURCE_DATE_EPOCH if set, and re-run when it changes.
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");

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
    emit("GAMUT_BUILD_TIMESTAMP", &build_timestamp());
}

/// Returns the build time as an RFC 3339 UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`). Honors
/// `SOURCE_DATE_EPOCH` (Unix seconds) for reproducible builds, falling back to the system
/// clock, and `unknown` if the clock is before the Unix epoch.
fn build_timestamp() -> String {
    let secs = match std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.trim().parse().ok())
    {
        Some(epoch) => epoch,
        None => match SystemTime::now().duration_since(UNIX_EPOCH) {
            Ok(dur) => dur.as_secs(),
            Err(_) => return "unknown".into(),
        },
    };
    format_rfc3339_utc(secs)
}

/// Formats Unix `secs` as an RFC 3339 UTC timestamp using the civil-from-days algorithm
/// (Howard Hinnant), avoiding a `chrono`/`time` build dependency.
fn format_rfc3339_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hour, min, sec) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    // Days since 1970-01-01 → civil (year, month, day). Shift epoch to 0000-03-01.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
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
