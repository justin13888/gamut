#!/bin/sh
# The one canonical `cargo mutants` invocation for this workspace.
#
# Every caller goes through here — `mise run mutants`, the mutants CI workflow, and an agent's
# tight debug loop — so the dials cannot drift between them. Run it via the mise task rather
# than directly; `mise run mutants --help` lists the flags.
#
# WHY THIS EXISTS
#
# Mutation testing multiplies the cost of a full `cargo test --all-features` by the mutant count
# (~24k across this workspace), and that test build compiles fourteen vendored C/C++ projects.
# Left to their defaults the parallelism dials all mean "as much as the machine has", which is
# the wrong number when memory runs out before cores do: the failure is the OOM killer, not a
# slow run. Worse, cargo-mutants bounds a mutant's *time* but never its *memory*, so a mutant
# that changes a dimension or a decode limit in a codec allocates without limit — and when the
# cgroup OOM killer answers that, it often picks `cargo mutants` itself, losing every result
# collected so far.
#
# So: derive every dial from a memory budget rather than the core count, put a hard ceiling
# under the whole run, and give each process an address-space limit so a runaway mutant aborts
# itself (scored `caught`) instead of taking the run down.
#
# ENVIRONMENT
#
#   GAMUT_MUTANTS_BUDGET_GB   memory budget; default: the cgroup limit, else MemAvailable
#   MUTANTS_JOBS              scenarios in flight            (derived)
#   MUTANTS_JOBSERVER_TASKS   compilers in flight, in total  (derived)
#   CARGO_BUILD_JOBS          cargo jobs per scenario; also NUM_JOBS for cc::Build  (derived)
#   CMAKE_BUILD_PARALLEL_LEVEL  compilers per vendored native build  (derived)
#   RUST_TEST_THREADS         test threads per scenario      (derived)
#   TMPDIR                    where build directories live   (default: target/mutants-tmp)
#   GAMUT_MUTANTS_NO_CGROUP=1 skip the systemd memory scope
#   GAMUT_MUTANTS_NO_ULIMIT=1 skip the per-process address-space limit
#
# Every derived value is overridable: set it in the environment and the derivation leaves it
# alone.

set -eu

die() {
	echo "mutants: $*" >&2
	exit 1
}

# ── Argument parsing ────────────────────────────────────────────────────────────────────────
# Selection is what bounds a run. `--crate`/`--diff`/`--file` narrow which mutants exist;
# `--shard` narrows which of them this process runs.

selection=""    # human-readable description of what was selected, for the banner
shard=""
iterate=""
budget_gb="${GAMUT_MUTANTS_BUDGET_GB:-}"
all_at_once=""
dry_run=""
set -- "$@"
args_pre=""     # cargo-mutants args accumulated before the passthrough marker

# Accumulate cargo-mutants arguments in a way that survives `sh`'s lack of arrays: each is
# appended to the positional list of a subshell-free loop via `set --`.
mutant_args=""
add_arg() {
	# Quote for `eval` below, so a glob or a space in a value survives intact.
	mutant_args="$mutant_args '$(printf '%s' "$1" | sed "s/'/'\\\\''/g")'"
}

usage() {
	cat <<'USAGE'
Usage: mise run mutants [OPTIONS] [-- CARGO_MUTANTS_ARGS...]

Selection (a full-workspace run requires --shard or --all-at-once):
  --diff                Only mutants in code changed vs the merge base with origin/master
  --crate NAME          Only mutants in one package (repeatable)
  --file GLOB           Only mutants in files matching a glob (repeatable)
  --shard I/N           Run shard I of N, round-robin across files
  --all-at-once         Permit an unsharded whole-workspace run

Loop:
  --iterate             Skip mutants caught in a previous run (for tight debug loops)

Resources:
  --budget GB           Memory budget; every parallelism dial is derived from it
  --dry-run             Resolve and print the invocation, then exit without running it

Anything after `--` is passed to cargo-mutants verbatim.
USAGE
}

while [ $# -gt 0 ]; do
	case "$1" in
	# A task runner expanding "no extra arguments" leaves an empty positional behind; that is
	# an absent argument, not an unknown option.
	"") ;;
	--diff)
		selection="${selection}diff "
		add_arg --in-diff
		add_arg "@@DIFF@@"
		;;
	--crate)
		[ $# -ge 2 ] || die "--crate needs a package name"
		selection="${selection}crate:$2 "
		add_arg --package
		add_arg "$2"
		shift
		;;
	--crate=*)
		selection="${selection}crate:${1#*=} "
		add_arg --package
		add_arg "${1#*=}"
		;;
	--file)
		[ $# -ge 2 ] || die "--file needs a glob"
		selection="${selection}file:$2 "
		add_arg --file
		add_arg "$2"
		shift
		;;
	--file=*)
		selection="${selection}file:${1#*=} "
		add_arg --file
		add_arg "${1#*=}"
		;;
	--shard)
		[ $# -ge 2 ] || die "--shard needs I/N"
		shard="$2"
		shift
		;;
	--shard=*) shard="${1#*=}" ;;
	--iterate) iterate=1 ;;
	--budget)
		[ $# -ge 2 ] || die "--budget needs a size in GiB"
		budget_gb="$2"
		shift
		;;
	--budget=*) budget_gb="${1#*=}" ;;
	--all-at-once) all_at_once=1 ;;
	--dry-run) dry_run=1 ;;
	-h | --help)
		usage
		exit 0
		;;
	--)
		shift
		while [ $# -gt 0 ]; do
			add_arg "$1"
			shift
		done
		break
		;;
	*) die "unknown option $1 (see --help)" ;;
	esac
	shift
done
: "${args_pre:=}"

# ── Memory budget ───────────────────────────────────────────────────────────────────────────
# The cgroup limit is the honest number where one exists: on a workstation running agents under
# a capped slice, or in a container, `free` reports the host's memory and the process is killed
# long before reaching it.

cgroup_limit_gb() {
	cgroup=$(awk -F: '$1 == "0" {print $3}' /proc/self/cgroup 2>/dev/null) || return 1
	[ -n "$cgroup" ] || return 1
	dir="/sys/fs/cgroup$cgroup"
	# Walk up to the nearest ancestor that names a finite limit; an unlimited level says nothing.
	while [ -n "$dir" ] && [ "$dir" != "/sys/fs/cgroup" ]; do
		if [ -r "$dir/memory.max" ]; then
			value=$(cat "$dir/memory.max")
			if [ "$value" != "max" ]; then
				echo $((value / 1024 / 1024 / 1024))
				return 0
			fi
		fi
		dir=$(dirname "$dir")
	done
	return 1
}

available_gb() {
	awk '/^MemAvailable:/ {print int($2 / 1024 / 1024)}' /proc/meminfo 2>/dev/null
}

if [ -z "$budget_gb" ]; then
	budget_gb=$(cgroup_limit_gb || true)
	budget_source="cgroup limit"
fi
if [ -z "$budget_gb" ] || [ "$budget_gb" -le 0 ] 2>/dev/null; then
	budget_gb=$(available_gb || true)
	budget_source="MemAvailable"
fi
[ -n "$budget_gb" ] && [ "$budget_gb" -gt 0 ] 2>/dev/null || {
	budget_gb=8
	budget_source="fallback"
}
: "${budget_source:=explicit}"

# Clamp. The floor is the 6 GiB a single package's build and test needs, which is also the
# per-process address-space floor below — going under it would leave the two guards inverted,
# with a process permitted more than the group it runs in. The ceiling is where more concurrency
# stops paying: the vendored C++ translation units are the bottleneck and do not divide further.
[ "$budget_gb" -lt 6 ] && budget_gb=6
[ "$budget_gb" -gt 32 ] && budget_gb=32

# ── Derived dials ───────────────────────────────────────────────────────────────────────────
# What has to be bounded is the number of C/C++ compilers alive at once, and that is a *product*
# of three dials, not any one of them: MUTANTS_JOBS scenarios, each running CARGO_BUILD_JOBS
# build scripts concurrently, each fanning out to CMAKE_BUILD_PARALLEL_LEVEL compilers. Bounding
# only the outer one is what makes "1 job" mean sixteen memory-hungry compiles.
#
# Anchored on a measured configuration: jobs 2, jobserver-tasks 4, CARGO_BUILD_JOBS 2 and
# CMAKE_BUILD_PARALLEL_LEVEL 2 — a product of 8 — peaks at ~12.8 GiB on a cold build of this
# workspace. So budget ~2 GiB per concurrent compiler and keep the product under that.

cores=$(nproc 2>/dev/null || echo 4)
slots=$((budget_gb / 2))
[ "$slots" -lt 1 ] && slots=1
[ "$slots" -gt "$cores" ] && slots="$cores"

# Scenarios first: more than four in flight stops paying, since each carries its own cold copy
# of the tree and they contend for the same disk.
jobs=$((slots / 4))
[ "$jobs" -lt 1 ] && jobs=1
[ "$jobs" -gt 4 ] && jobs=4

# Split what one scenario may use between cargo's job count and each build script's fan-out, so
# their product — not either factor — is the per-scenario compiler budget.
per_job=$((slots / jobs))
[ "$per_job" -lt 1 ] && per_job=1
cargo_jobs=1
while [ $(((cargo_jobs + 1) * (cargo_jobs + 1))) -le "$per_job" ]; do
	cargo_jobs=$((cargo_jobs + 1))
done
cmake_level=$((per_job / cargo_jobs))
[ "$cmake_level" -lt 1 ] && cmake_level=1

MUTANTS_JOBS="${MUTANTS_JOBS:-$jobs}"
MUTANTS_JOBSERVER_TASKS="${MUTANTS_JOBSERVER_TASKS:-$slots}"
CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-$cargo_jobs}"
CMAKE_BUILD_PARALLEL_LEVEL="${CMAKE_BUILD_PARALLEL_LEVEL:-$cmake_level}"
# The test harness defaults to one thread per core and each thread decodes images, so it is a
# third unbounded multiplier if left alone.
RUST_TEST_THREADS="${RUST_TEST_THREADS:-$per_job}"
export CARGO_BUILD_JOBS CMAKE_BUILD_PARALLEL_LEVEL RUST_TEST_THREADS

# ── Build directories ───────────────────────────────────────────────────────────────────────
# cargo-mutants puts each job's copy of the tree under TMPDIR. Two hazards: a tmpfs TMPDIR
# charges every build directory to memory (and /tmp is a tmpfs on many systems, sometimes under
# a bind mount that is absent inside a mount namespace), and each job's directory grows a cold
# target/ of its own.

target_dir="${CARGO_TARGET_DIR:-target}"
if [ -z "${TMPDIR:-}" ]; then
	TMPDIR="$target_dir/mutants-tmp"
	mkdir -p "$TMPDIR"
fi
# Absolute: children run with a working directory of their own (each mutant builds in a copy of
# the tree), so a relative TMPDIR would resolve somewhere different for each of them.
TMPDIR=$(cd "$TMPDIR" && pwd) || die "TMPDIR=$TMPDIR is not a usable directory"
export TMPDIR

fstype=$(stat -f -c '%T' "$TMPDIR" 2>/dev/null || echo unknown)
case "$fstype" in
tmpfs | ramfs)
	die "TMPDIR=$TMPDIR is $fstype — every build directory would be charged to memory.
     Point TMPDIR at a real filesystem (unset it to use $target_dir/mutants-tmp)."
	;;
esac

# Each job's build directory holds a cold target/ for this workspace; measured at ~13 GiB warm.
free_gb=$(df -BG --output=avail "$TMPDIR" 2>/dev/null | tail -1 | tr -dc '0-9')
need_gb=$((MUTANTS_JOBS * 15))
if [ -n "$free_gb" ] && [ "$free_gb" -lt "$need_gb" ]; then
	die "$TMPDIR has ${free_gb}GiB free; $MUTANTS_JOBS job(s) need about ${need_gb}GiB.
     Free space, set TMPDIR to a roomier filesystem, or lower MUTANTS_JOBS."
fi

# ── Selection guard ─────────────────────────────────────────────────────────────────────────
# An unsharded whole-workspace run is ~24k mutants. It is not a thing to start by accident:
# it runs for days, and the longer it runs the likelier it is to meet the one mutant that
# allocates without limit.

if [ -z "$selection" ] && [ -z "$shard" ] && [ -z "$all_at_once" ]; then
	count=$(cargo mutants --list 2>/dev/null | wc -l | tr -d ' ')
	cat >&2 <<EOF
mutants: refusing an unsharded whole-workspace run of $count mutants.

Every mutant is a full build and test of its package with all features on, which for this
workspace includes the vendored C/C++ oracles. At $MUTANTS_JOBS job(s) this runs for days, and
a run that long is the one most likely to meet a mutant that allocates without limit.

Pick a bound:
  mise run mutants -- --diff                 only what this branch changed (what PR CI runs)
  mise run mutants -- --crate gamut-png      one package
  mise run mutants -- --shard 1/16           one shard of the whole workspace
  mise run mutants -- --all-at-once          you meant it
EOF
	exit 2
fi

[ -n "$shard" ] && {
	add_arg --shard
	add_arg "$shard"
	add_arg --sharding
	# Round-robin, not contiguous slices: mutants from one file land in one slice, so a file
	# with slow tests would load a single shard while the others idle.
	add_arg round-robin
	selection="${selection}shard:$shard "
}
[ -n "$iterate" ] && add_arg --iterate

# The diff is materialised here rather than by the caller so every entry point spells the merge
# base the same way.
if [ "${mutant_args#*@@DIFF@@}" != "$mutant_args" ]; then
	mkdir -p "$target_dir"
	diff_file="$target_dir/mutants.diff"
	base=$(git merge-base origin/master HEAD) || die "cannot find the merge base with origin/master"
	git diff "$base...HEAD" >"$diff_file"
	[ -s "$diff_file" ] || die "no changes vs origin/master, so --diff selects no mutants"
	escaped=$(printf '%s' "$diff_file" | sed "s/'/'\\\\''/g")
	mutant_args=$(printf '%s' "$mutant_args" | sed "s|@@DIFF@@|$escaped|")
fi

# ── Memory guards ───────────────────────────────────────────────────────────────────────────
# Two layers, because one is not enough. The cgroup ceiling stops the run from taking the
# machine down, but the cgroup OOM killer picks the largest process in the group, which is as
# likely to be cargo-mutants as the mutant that misbehaved — and killing cargo-mutants throws
# away every result. The per-process address-space limit is what makes the runaway abort itself
# first, so it is scored `caught` and the run continues.

ulimit_kb=$((budget_gb * 1024 * 1024 / MUTANTS_JOBS))
floor_kb=$((6 * 1024 * 1024)) # a full build+test of one package fits in 6 GiB of address space
[ "$ulimit_kb" -lt "$floor_kb" ] && ulimit_kb="$floor_kb"

inner="cargo mutants --jobs $MUTANTS_JOBS --jobserver-tasks $MUTANTS_JOBSERVER_TASKS$mutant_args"
if [ -z "${GAMUT_MUTANTS_NO_ULIMIT:-}" ]; then
	inner="ulimit -v $ulimit_kb; exec $inner"
	guard_note="ulimit -v $((ulimit_kb / 1024 / 1024))GiB per process"
else
	inner="exec $inner"
	guard_note="none (GAMUT_MUTANTS_NO_ULIMIT)"
fi

scope=""
# `is-system-running` exits non-zero for "degraded" — one unrelated failed unit — which says
# nothing about whether the manager can start a scope. Probe for a manager that answers at all.
user_systemd=$(systemctl --user is-system-running 2>/dev/null || true)
case "$user_systemd" in
"" | offline | unknown) user_systemd="" ;;
esac
if [ -z "${GAMUT_MUTANTS_NO_CGROUP:-}" ] &&
	command -v systemd-run >/dev/null 2>&1 &&
	[ -n "$user_systemd" ]; then
	# MemorySwapMax=0: swapping a build this large does not recover it, it only makes the
	# machine unusable for everyone else on the way down.
	scope="systemd-run --user --scope -q -p MemoryMax=${budget_gb}G -p MemorySwapMax=0 \
--unit=gamut-mutants-$$ --"
	guard_note="$guard_note, MemoryMax=${budget_gb}G (gamut-mutants-$$.scope)"
fi

# ── Go ──────────────────────────────────────────────────────────────────────────────────────
# Echo the resolved configuration: a CI log or an agent transcript should record exactly what
# ran, without anyone having to re-derive it.

cat >&2 <<EOF
mutants: selection   ${selection:-whole workspace}
mutants: budget      ${budget_gb}GiB ($budget_source), $cores core(s)
mutants: jobs        $MUTANTS_JOBS scenario(s), $MUTANTS_JOBSERVER_TASKS jobserver task(s)
mutants: per job     CARGO_BUILD_JOBS=$CARGO_BUILD_JOBS CMAKE_BUILD_PARALLEL_LEVEL=$CMAKE_BUILD_PARALLEL_LEVEL RUST_TEST_THREADS=$RUST_TEST_THREADS
mutants: tmpdir      $TMPDIR ($fstype, ${free_gb:-?}GiB free)
mutants: guards      $guard_note
mutants: command     ${scope:+$scope }sh -c "${inner#ulimit*; }"
EOF

[ -n "$dry_run" ] && exit 0

# shellcheck disable=SC2086 # $scope is an intentionally word-split command prefix
exec $scope sh -c "$inner"
