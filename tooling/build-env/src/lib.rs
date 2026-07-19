//! Normalises the ambient C/C++ toolchain environment so the vendored oracle builds are
//! *hermetic to exactly what they configure*.
//!
//! # The problem
//!
//! The oracle build scripts shell out to `cmake`/`meson` and let those tools detect the
//! compiler from the environment. A developer whose shell exports a compiler cache breaks
//! that detection in two independent ways:
//!
//! 1. **Launcher doubling.** With `CC="sccache gcc"` *and* `CMAKE_C_COMPILER_LAUNCHER=sccache`,
//!    CMake splits `CC` into `CMAKE_C_COMPILER=sccache` and *then* prefixes the launcher again,
//!    producing `sccache sccache gcc` — a recursive invocation sccache rejects.
//! 2. **Bare-compiler execution.** libpng's generated `genout.cmake` runs the detected compiler
//!    bare — `execute_process(COMMAND "${CMAKE_C_COMPILER}" "-E" ...)`, with no launcher
//!    expansion. A `CMAKE_C_COMPILER` of `sccache` fails there even when ordinary compilation
//!    succeeded, because `sccache -E ...` has no compiler to delegate to.
//!
//! # The fix
//!
//! Both failures are caused by launcher *misplacement*, not by the launcher's existence. This
//! crate moves the launcher to the one position CMake defines for it:
//!
//! ```text
//! CC="sccache gcc"                 ->  CC=gcc
//!                                      CMAKE_C_COMPILER_LAUNCHER=sccache   (exactly once)
//! ```
//!
//! so compile rules run `sccache gcc -c foo.c` (still cached) while `genout.cmake` runs a bare,
//! `-E`-capable `gcc`. A compiler cache is a transparent accelerator — it changes neither build
//! inputs nor outputs — so preserving it does not weaken the hermeticity the oracles document.
//!
//! # Scope
//!
//! Changes are applied **per spawned [`Command`]**, never to this process's environment
//! (`std::env::set_var` is `unsafe` in edition 2024, and mutating a build script's global env
//! would leak into unrelated work). A developer's shell configuration is never modified.
//!
//! Set `GAMUT_BUILD_KEEP_ENV=1` to disable all of this and use the ambient environment verbatim.
//!
//! # What this deliberately does not cover
//!
//! Only build scripts that shell out to **`cmake`/`meson` directly** need this. Those tools get
//! no `-DCMAKE_C_COMPILER`, so CMake parses `CC` itself — and CMake takes the *first word* as the
//! compiler, yielding `CMAKE_C_COMPILER=sccache`, which is what triggers both failures above.
//!
//! Build scripts driving the `cc` or `cmake` **crates** need no help, and are deliberately left
//! alone:
//!
//! - `cc` resolves a launcher-prefixed `CC` into a program plus arguments and never re-applies a
//!   launcher on top.
//! - `cmake` passes `cc`'s *resolved* compiler path, so `CC="sccache gcc"` reaches CMake as
//!   `CMAKE_C_COMPILER=/usr/bin/gcc` with the launcher already in its proper place.
//!
//! This is verified rather than assumed: `crates/gamut-jxl-sys` (the `cmake` crate) and
//! `tooling/{lcms2,gamut-dng}-oracle` (`cc::Build`) build correctly under a launcher-prefixed
//! environment with no involvement from this crate.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Opt-out switch: use the ambient compiler environment unmodified.
pub const KEEP_ENV_VAR: &str = "GAMUT_BUILD_KEEP_ENV";

/// Argv-0 basenames recognised as compiler launchers. Compared against the file *stem* of the
/// first whitespace-separated word of `CC`/`CXX`, so both `sccache` and `/usr/bin/sccache.exe`
/// match. Anything else is treated as the compiler itself.
const LAUNCHERS: &[&str] = &[
    "sccache",
    "ccache",
    "distcc",
    "buildcache",
    "icecc",
    "icerun",
];

/// The two languages whose toolchain variables are normalised.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Lang {
    C,
    Cxx,
}

impl Lang {
    /// The base environment variable name (`CC` / `CXX`).
    const fn var(self) -> &'static str {
        match self {
            Lang::C => "CC",
            Lang::Cxx => "CXX",
        }
    }

    /// The CMake per-language launcher variable.
    const fn cmake_launcher_var(self) -> &'static str {
        match self {
            Lang::C => "CMAKE_C_COMPILER_LAUNCHER",
            Lang::Cxx => "CMAKE_CXX_COMPILER_LAUNCHER",
        }
    }
}

/// A normalised toolchain environment, resolved once from the ambient environment and then
/// applied to each command the build script spawns.
///
/// Construct with [`BuildEnv::detect`]; apply with [`BuildEnv::apply`] (or the free function
/// [`sanitize`], which is the one-liner most build scripts want).
#[derive(Clone, Debug, Default)]
pub struct BuildEnv {
    /// Variables to set on each spawned command, in insertion order.
    vars: Vec<(String, String)>,
    /// A launcher-shim-filtered `PATH`, when filtering removed anything.
    path: Option<OsString>,
    /// Whether the opt-out was honoured (everything else is then empty).
    disabled: bool,
}

impl BuildEnv {
    /// Resolves the normalised toolchain for this build script's target from the ambient
    /// environment.
    ///
    /// Emits the `cargo:rerun-if-env-changed` lines for every variable consulted, so cargo
    /// re-runs the build script when the developer's toolchain configuration changes. Cheap —
    /// call once per build script.
    ///
    /// # Panics
    ///
    /// Panics if `CC`/`CXX` names a launcher with no compiler after it (e.g. `CC=sccache`).
    /// That is a malformed configuration which cannot be repaired by guessing, and a build
    /// script has no better error channel.
    #[must_use]
    pub fn detect() -> Self {
        let target = std::env::var("TARGET").unwrap_or_default();
        let host = std::env::var("HOST").unwrap_or_default();
        let path = std::env::var_os("PATH");

        for var in rerun_vars(&target, &host) {
            println!("cargo:rerun-if-env-changed={var}");
        }

        if std::env::var(KEEP_ENV_VAR).as_deref() == Ok("1") {
            println!(
                "cargo:warning={KEEP_ENV_VAR}=1: using the ambient compiler environment unmodified"
            );
            return Self {
                disabled: true,
                ..Self::default()
            };
        }

        let ambient: BTreeMap<String, String> = std::env::vars().collect();
        Self::resolve(&ambient, &target, &host, path.as_deref())
    }

    /// The pure core of [`BuildEnv::detect`], factored out so the unit tests never touch the
    /// real process environment.
    fn resolve(
        ambient: &BTreeMap<String, String>,
        target: &str,
        host: &str,
        path: Option<&OsStr>,
    ) -> Self {
        let mut vars = Vec::new();
        let mut launcher_active = false;

        for lang in [Lang::C, Lang::Cxx] {
            // Only the variable `cc`/cmake would actually pick is examined, and it is rewritten
            // under its own name. Rewriting bare `CC` while `CC_<triple>` is the winner would be
            // a silent no-op; writing into a different name would clobber the target override.
            let Some((name, value)) = winning_var(ambient, lang, target, host) else {
                continue;
            };

            // An ambient launcher variable that is already correctly positioned is left alone.
            if ambient.contains_key(lang.cmake_launcher_var()) {
                launcher_active = true;
            }

            let Some((launcher, compiler)) = split_launcher(&value, &name) else {
                continue;
            };

            launcher_active = true;
            vars.push((name, compiler));
            // Set the launcher in the one position CMake defines for it. If it was also set
            // ambiently this overwrites it with the same value, which is what makes the
            // "exactly once" guarantee hold rather than doubling.
            vars.push((lang.cmake_launcher_var().to_owned(), launcher));
        }

        // A `ccache` shim directory on PATH is a *second*, implicit launcher: `gcc` already
        // resolves to a cache wrapper. Combined with an explicit launcher that is unconditional
        // double-caching, and it cannot be normalised into a single well-defined position — so
        // when a launcher is active, drop the shims and let the explicit one do the work.
        let path = if launcher_active {
            path.and_then(filter_launcher_shims)
        } else {
            None
        };

        Self {
            vars,
            path,
            disabled: false,
        }
    }

    /// Applies the normalised toolchain to a command that is about to be spawned.
    ///
    /// Idempotent, and a no-op when nothing needed normalising or when [`KEEP_ENV_VAR`] is set.
    ///
    /// A `PATH` the caller set on the command explicitly is **never** overwritten — build
    /// scripts that prepend a vendored tool directory (`path_with_nasm`) set `PATH` while
    /// building the command, and this runs afterwards at the spawn chokepoint. Those callers
    /// should base their `PATH` on [`BuildEnv::path`] so the shim filtering still applies.
    pub fn apply<'c>(&self, cmd: &'c mut Command) -> &'c mut Command {
        for (key, value) in &self.vars {
            cmd.env(key, value);
        }
        if let Some(path) = &self.path
            && !overrides_env(cmd, "PATH")
        {
            cmd.env("PATH", path);
        }
        cmd
    }

    /// The launcher-shim-filtered `PATH`, for build scripts that do their own `PATH` munging.
    ///
    /// Use this as the base instead of `std::env::var_os("PATH")`: a later per-command
    /// `.env("PATH", ...)` would otherwise reinstate the shims this stripped.
    #[must_use]
    pub fn path(&self) -> OsString {
        self.path
            .clone()
            .or_else(|| std::env::var_os("PATH"))
            .unwrap_or_default()
    }

    /// Whether the [`KEEP_ENV_VAR`] opt-out was honoured.
    #[must_use]
    pub fn is_disabled(&self) -> bool {
        self.disabled
    }
}

/// Resolves and applies the normalised toolchain to a single command.
///
/// The one-liner for a build script's existing spawn chokepoint:
///
/// ```ignore
/// fn run(cmd: &mut Command) {
///     build_env::sanitize(cmd);
///     // ... existing spawn + status check
/// }
/// ```
///
/// Prefer caching a [`BuildEnv::detect`] result when spawning many commands; this re-resolves
/// each call, which is cheap but not free (and re-emits the `rerun-if-env-changed` lines,
/// which cargo deduplicates).
pub fn sanitize(cmd: &mut Command) -> &mut Command {
    BuildEnv::detect().apply(cmd)
}

/// Whether `cmd` already has an explicit override for `key`.
fn overrides_env(cmd: &Command, key: &str) -> bool {
    cmd.get_envs()
        .any(|(k, v)| k == OsStr::new(key) && v.is_some())
}

/// Every environment variable [`BuildEnv::detect`] may consult, for `rerun-if-env-changed`.
fn rerun_vars(target: &str, host: &str) -> Vec<String> {
    let mut vars = vec![KEEP_ENV_VAR.to_owned(), "PATH".to_owned()];
    for lang in [Lang::C, Lang::Cxx] {
        vars.extend(candidate_names(lang, target, host));
        vars.push(lang.cmake_launcher_var().to_owned());
    }
    vars
}

/// The candidate variable names for `lang`, in the order the `cc` crate resolves them.
fn candidate_names(lang: Lang, target: &str, host: &str) -> Vec<String> {
    let base = lang.var();
    let mut names = Vec::new();
    if !target.is_empty() {
        names.push(format!("{base}_{target}"));
        names.push(format!("{base}_{}", target.replace('-', "_")));
    }
    names.push(format!(
        "{}_{base}",
        if !target.is_empty() && target == host {
            "HOST"
        } else {
            "TARGET"
        }
    ));
    names.push(base.to_owned());
    names.dedup();
    names
}

/// The first candidate variable that is actually set, with its value.
fn winning_var(
    ambient: &BTreeMap<String, String>,
    lang: Lang,
    target: &str,
    host: &str,
) -> Option<(String, String)> {
    candidate_names(lang, target, host)
        .into_iter()
        .find_map(|name| ambient.get(&name).map(|v| (name, v.clone())))
}

/// Splits a `CC`-style value into `(launcher, compiler)` when it is launcher-prefixed.
///
/// Returns `None` when the value is a plain compiler (the common case).
///
/// # Panics
///
/// Panics when the value is a launcher with nothing after it — a malformed configuration that
/// cannot be repaired by guessing which compiler was meant.
fn split_launcher(value: &str, var_name: &str) -> Option<(String, String)> {
    let mut words = value.split_whitespace();
    let first = words.next()?;
    if !is_launcher(first) {
        return None;
    }
    let rest = words.collect::<Vec<_>>().join(" ");
    assert!(
        !rest.is_empty(),
        "{var_name}={value:?} names the compiler launcher {first:?} with no compiler after it. \
         Set it to the launcher followed by the compiler (e.g. {var_name}=\"{first} gcc\"), or \
         set {var_name} to the bare compiler and let the build position the launcher itself."
    );
    Some((first.to_owned(), rest))
}

/// Whether `word` names a known compiler launcher, comparing the file stem so that both
/// `sccache` and `/usr/bin/sccache.exe` match.
fn is_launcher(word: &str) -> bool {
    let stem = Path::new(word)
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    LAUNCHERS.contains(&stem.as_str())
}

/// Drops `ccache`-style shim directories from a `PATH`, returning `None` when nothing changed.
fn filter_launcher_shims(path: &OsStr) -> Option<OsString> {
    let entries: Vec<PathBuf> = std::env::split_paths(path).collect();
    let kept: Vec<&PathBuf> = entries.iter().filter(|p| !is_shim_dir(p)).collect();
    if kept.len() == entries.len() {
        return None;
    }
    std::env::join_paths(kept).ok()
}

/// Whether `dir` is a compiler-cache shim directory — `/usr/lib64/ccache`, `/usr/lib/ccache`,
/// or a `libexec` directory under one (Homebrew's layout).
fn is_shim_dir(dir: &Path) -> bool {
    let leaf = dir
        .file_name()
        .map(|s| s.to_string_lossy().to_ascii_lowercase());
    match leaf.as_deref() {
        Some("ccache") => true,
        Some("libexec") => dir
            .parent()
            .and_then(Path::file_name)
            .map(|s| s.to_string_lossy().to_ascii_lowercase() == "ccache")
            .unwrap_or(false),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    fn var<'a>(be: &'a BuildEnv, key: &str) -> Option<&'a str> {
        be.vars
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn splits_launcher_into_bare_compiler_and_cmake_launcher() {
        let be = BuildEnv::resolve(
            &env(&[("CC", "sccache gcc"), ("CXX", "sccache g++")]),
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        );
        assert_eq!(var(&be, "CC"), Some("gcc"));
        assert_eq!(var(&be, "CXX"), Some("g++"));
        assert_eq!(var(&be, "CMAKE_C_COMPILER_LAUNCHER"), Some("sccache"));
        assert_eq!(var(&be, "CMAKE_CXX_COMPILER_LAUNCHER"), Some("sccache"));
    }

    /// The doubling case: an ambient launcher variable is overwritten with the same value
    /// rather than left to stack on top of a launcher-prefixed `CC`.
    #[test]
    fn ambient_launcher_var_is_set_exactly_once() {
        let be = BuildEnv::resolve(
            &env(&[
                ("CC", "sccache gcc"),
                ("CMAKE_C_COMPILER_LAUNCHER", "sccache"),
            ]),
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        );
        assert_eq!(var(&be, "CC"), Some("gcc"));
        let launchers: Vec<_> = be
            .vars
            .iter()
            .filter(|(k, _)| k == "CMAKE_C_COMPILER_LAUNCHER")
            .collect();
        assert_eq!(launchers.len(), 1, "launcher must be set exactly once");
        assert_eq!(launchers[0].1, "sccache");
    }

    /// A bare `CC` with a separate launcher variable is already correct — CMake compiles with
    /// `sccache gcc` and genout runs a bare `gcc`. Nothing to repair.
    #[test]
    fn bare_compiler_with_launcher_var_is_left_alone() {
        let be = BuildEnv::resolve(
            &env(&[("CC", "gcc"), ("CMAKE_C_COMPILER_LAUNCHER", "sccache")]),
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        );
        assert!(
            be.vars.is_empty(),
            "expected no rewrites, got {:?}",
            be.vars
        );
    }

    #[test]
    fn clean_environment_is_untouched() {
        let be = BuildEnv::resolve(
            &env(&[("CC", "clang")]),
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Some(OsStr::new("/usr/bin:/bin")),
        );
        assert!(be.vars.is_empty());
        assert!(be.path.is_none());
    }

    /// Load-bearing for cross builds: the target-specific variable is the one `cc` picks, so it
    /// is the one that must be rewritten — and bare `CC` must not be touched.
    #[test]
    fn rewrites_the_target_specific_variable_only() {
        let be = BuildEnv::resolve(
            &env(&[
                ("CC", "sccache gcc"),
                (
                    "CC_aarch64-unknown-linux-musl",
                    "sccache aarch64-linux-gnu-gcc",
                ),
            ]),
            "aarch64-unknown-linux-musl",
            "x86_64-unknown-linux-gnu",
            None,
        );
        assert_eq!(
            var(&be, "CC_aarch64-unknown-linux-musl"),
            Some("aarch64-linux-gnu-gcc")
        );
        assert_eq!(var(&be, "CC"), None, "bare CC must not be rewritten");
    }

    #[test]
    fn underscored_target_variable_is_recognised() {
        let be = BuildEnv::resolve(
            &env(&[("CC_aarch64_unknown_linux_musl", "ccache clang")]),
            "aarch64-unknown-linux-musl",
            "x86_64-unknown-linux-gnu",
            None,
        );
        assert_eq!(var(&be, "CC_aarch64_unknown_linux_musl"), Some("clang"));
    }

    #[test]
    fn launcher_path_is_recognised_by_stem() {
        assert!(is_launcher("/usr/bin/sccache"));
        assert!(is_launcher("sccache.exe"));
        assert!(is_launcher("CCache"));
        assert!(!is_launcher("gcc"));
        assert!(!is_launcher("/usr/bin/clang-18"));
    }

    #[test]
    fn ccache_shim_dirs_are_dropped_only_when_a_launcher_is_active() {
        let dirty = OsStr::new("/usr/lib64/ccache:/usr/bin:/bin");
        let be = BuildEnv::resolve(
            &env(&[("CC", "sccache gcc")]),
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Some(dirty),
        );
        assert_eq!(be.path.as_deref(), Some(OsStr::new("/usr/bin:/bin")));

        // No launcher anywhere: the shim is the developer's only cache and is correctly
        // positioned, so it stays.
        let be = BuildEnv::resolve(
            &env(&[("CC", "gcc")]),
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Some(dirty),
        );
        assert!(be.path.is_none());
    }

    #[test]
    fn homebrew_ccache_libexec_shim_is_dropped() {
        assert!(is_shim_dir(Path::new("/usr/local/opt/ccache/libexec")));
        assert!(is_shim_dir(Path::new("/usr/lib/ccache")));
        assert!(!is_shim_dir(Path::new("/usr/libexec")));
        assert!(!is_shim_dir(Path::new("/usr/bin")));
    }

    #[test]
    #[should_panic(expected = "with no compiler after it")]
    fn launcher_with_no_compiler_is_rejected() {
        let _ = BuildEnv::resolve(
            &env(&[("CC", "sccache")]),
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        );
    }

    /// `run` sanitizes at the spawn chokepoint, i.e. *after* callers like `path_with_nasm` have
    /// already put a vendored tool directory on the command's `PATH`. Overwriting it there would
    /// silently drop the vendored nasm and break the x86 SIMD assembly.
    #[test]
    fn apply_does_not_clobber_a_path_the_command_already_set() {
        let be = BuildEnv::resolve(
            &env(&[("CC", "sccache gcc")]),
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            Some(OsStr::new("/usr/lib64/ccache:/usr/bin")),
        );
        assert!(
            be.path.is_some(),
            "test presumes filtering had something to do"
        );

        let mut cmd = Command::new("true");
        cmd.env("PATH", "/vendored/nasm:/usr/bin");
        be.apply(&mut cmd);

        let path = cmd
            .get_envs()
            .find(|(k, _)| *k == OsStr::new("PATH"))
            .and_then(|(_, v)| v);
        assert_eq!(path, Some(OsStr::new("/vendored/nasm:/usr/bin")));
    }

    #[test]
    fn apply_sets_the_rewritten_vars_on_the_command() {
        let be = BuildEnv::resolve(
            &env(&[("CC", "sccache gcc")]),
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-gnu",
            None,
        );
        let mut cmd = Command::new("true");
        be.apply(&mut cmd);
        let envs: Vec<_> = cmd.get_envs().collect();
        assert!(
            envs.contains(&(OsStr::new("CC"), Some(OsStr::new("gcc")))),
            "got {envs:?}"
        );
        assert!(envs.contains(&(
            OsStr::new("CMAKE_C_COMPILER_LAUNCHER"),
            Some(OsStr::new("sccache"))
        )));
    }
}
