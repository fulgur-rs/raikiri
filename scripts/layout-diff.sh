#!/usr/bin/env bash
# Compare layout results between the merge-base and HEAD.
#
# Usage: scripts/layout-diff.sh [--base <ref>] [--seeds <n>] [--wpt-baseline]
#                               [--wpt <dir>]... [--font <file>] [--show <n>]
#                               [--timeout <s>] [--no-cache]
#
#   --base <ref>     Ref whose merge-base with HEAD is the baseline
#                    (default: origin/main).
#   --seeds <n>      Generated cases use seeds 0..n (default: 2000). A seed
#                    stands for a small document mixing out-of-flow boxes,
#                    vertical writing modes, multi-column containers, tables,
#                    flex and grid containers (see `generate.rs`).
#   --wpt-baseline   Add every test of expectations/raikiri-baseline.txt and
#                    expectations/expected-failures.txt (HEAD's copies) as a
#                    case, about 4,700 files of the pinned WPT checkout.
#   --wpt <dir>      Add every .html file under this directory of the WPT
#                    checkout (for example css/css-flexbox) as a case.
#                    Repeatable.
#   --font <file>    The font cases are laid out in, registered as `Ahem`
#                    (default: the Ahem copy under crates/raikiri-dom/tests/
#                    data). Installed fonts play no part, so the result does
#                    not depend on the machine.
#   --show <n>       Print a field-level diff for the first n differing cases
#                    (default: 3).
#   --timeout <s>    Stop a build that has not run every case after this many
#                    seconds (default: 3600).
#   --no-cache       Build the merge-base again even when a build of it is
#                    cached (see below).
#
# Builds crates/raikiri-layout-diff twice — against a detached checkout of
# the merge-base and against the current tree — runs both builds on the same
# cases, and reports every case whose canonical layout output differs. The
# output holds the page API surface (fragments, text runs, paint order, margin
# boxes, links, root box), the layout of every node as f32 bits and what the
# style getters of every box return, so a box that moves by a thousandth of a
# pixel, or a getter that reads a value differently, is reported. A change
# that is meant to preserve layout must report none. Cases that panic are
# compared like any other output, so a new panic is reported as a difference.
# Both builds enable debug assertions and overflow checks, so a broken
# internal invariant shows up as a panic as well.
#
# Not part of scripts/gate.sh: a run takes minutes and is for changes that
# claim to preserve layout. See crates/raikiri-layout-diff/README.md.
#
# WPT cases are parsed at their path under the WPT checkout, which also serves
# their linked stylesheets and fonts, so root-relative links such as
# /css/support/... resolve.
#
# The merge-base build is cached under $RAIKIRI_LAYOUT_DIFF_CACHE (default:
# ~/.cache/raikiri/layout-diff), keyed by the merge-base commit, the tool's
# sources and manifest, the compiler cargo built the head with, and the
# build settings from the environment and cargo configuration, so repeated
# runs against one base skip building it. A cached build is only reused
# while the environment variables its build scripts and crates declared that
# they read keep the values they had, as cargo would only reuse it then, and
# only when that compiler built it. The merge-base checkout is a throwaway
# worktree under the main checkout's .worktrees/, removed when the run ends.
#
# Exit status: 0 when no case differs, 1 when some case differs, 2 on a usage
# or tooling error, including a build that crashes or hangs on a case (the
# case is named).

set -Eeuo pipefail
# The trap also runs in subshells, which only pass the failure on; the
# command that failed at the top level is the one reported.
trap 'if [[ $BASH_SUBSHELL -eq 0 ]]; then echo "layout-diff.sh: line $LINENO: \`$BASH_COMMAND\` failed" >&2; fi; exit 2' ERR

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/repo_root.sh
source "$SCRIPT_DIR/lib/repo_root.sh"
# shellcheck source=wpt/lib/cache_path.sh
source "$SCRIPT_DIR/wpt/lib/cache_path.sh"
cd "$REPO_ROOT"
# shellcheck source=lib/tmpdir.sh
source "$SCRIPT_DIR/lib/tmpdir.sh"

BASE_REF="origin/main"
SEEDS=2000
SHOW=3
TIMEOUT=3600
USE_CACHE=1
WPT_DIRS=()
WPT_BASELINE=0
FONT="$REPO_ROOT/crates/raikiri-dom/tests/data/text-autospace/Ahem.ttf"

usage_error() {
  echo "layout-diff.sh: $1" >&2
  exit 2
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base) [[ $# -ge 2 ]] || usage_error "--base needs a ref"; BASE_REF="$2"; shift 2 ;;
    --seeds) [[ $# -ge 2 ]] || usage_error "--seeds needs a count"; SEEDS="$2"; shift 2 ;;
    --wpt) [[ $# -ge 2 ]] || usage_error "--wpt needs a directory"; WPT_DIRS+=("$2"); shift 2 ;;
    --wpt-baseline) WPT_BASELINE=1; shift ;;
    --font) [[ $# -ge 2 ]] || usage_error "--font needs a file"; FONT="$2"; shift 2 ;;
    --show) [[ $# -ge 2 ]] || usage_error "--show needs a count"; SHOW="$2"; shift 2 ;;
    --timeout) [[ $# -ge 2 ]] || usage_error "--timeout needs seconds"; TIMEOUT="$2"; shift 2 ;;
    --no-cache) USE_CACHE=0; shift ;;
    *) usage_error "unknown argument: $1" ;;
  esac
done
[[ "$SEEDS" =~ ^[0-9]{1,7}$ ]] || usage_error "--seeds must be a non-negative integer"
[[ "$SHOW" =~ ^[0-9]{1,4}$ ]] || usage_error "--show must be a non-negative integer"
[[ "$TIMEOUT" =~ ^[1-9][0-9]{0,6}$ ]] || usage_error "--timeout must be a positive integer"
[[ -f "$FONT" ]] || usage_error "--font: $FONT is not a file"
[[ "$SEEDS" -gt 0 || "$WPT_BASELINE" -eq 1 || ${#WPT_DIRS[@]} -gt 0 ]] \
  || usage_error "no cases: give --seeds, --wpt-baseline or --wpt"

BASE_SHA="$(git merge-base "$BASE_REF" HEAD)"
SCRATCH="$(mktemp -d "$TMPDIR/raikiri-layout-diff.XXXXXX")"
# Throwaway worktrees that run cargo live under the main checkout's
# .worktrees/, whichever worktree this runs in.
MAIN_ROOT="$(dirname "$(git rev-parse --path-format=absolute --git-common-dir)")"
BASE_TREE="$MAIN_ROOT/.worktrees/layout-diff-base-$$-$RANDOM"
CACHE_ROOT="${RAIKIRI_LAYOUT_DIFF_CACHE:-${XDG_CACHE_HOME:-$HOME/.cache}/raikiri/layout-diff}"
# Cached base builds kept, most recently used first.
CACHE_KEEP=8

DUMP_PIDS=()

cleanup() {
  # A failure while cleaning up must not replace the run's exit status.
  trap - ERR
  # Dumps still running when the run ends early.
  if [[ ${#DUMP_PIDS[@]} -gt 0 ]]; then
    kill "${DUMP_PIDS[@]}" 2>/dev/null || true
  fi
  if [[ -d "$BASE_TREE" ]]; then
    git -C "$REPO_ROOT" worktree remove --force "$BASE_TREE" >/dev/null 2>&1 || true
  fi
  rm -rf "$SCRATCH" || true
}
trap cleanup EXIT
# Background jobs ignore the interrupt signal, so the run ends on it itself.
trap 'exit 130' INT TERM

echo "== scripts/layout-diff.sh =="
echo "head      : $(git rev-parse HEAD)"
echo "base ref  : $BASE_REF"
echo "base sha  : $BASE_SHA"
echo "seeds     : 0..$SEEDS"
echo "TMPDIR    : $TMPDIR"
echo "cache     : $CACHE_ROOT"
echo

# Prints the manifest the tool is built with against the crates of tree $1.
# Its dependencies mirror crates/raikiri-layout-diff/Cargo.toml. The tree's
# lockfile pins the version of taffy; the features listed are those the tool's
# own code needs, and the features of the tree's crates are added to them.
tool_manifest() {
  local tree="$1"
  cat <<EOF
[package]
name = "raikiri-layout-diff"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
raikiri-dom = { path = "$tree/crates/raikiri-dom" }
raikiri-html = { path = "$tree/crates/raikiri-html" }
raikiri-layout-style = { path = "$tree/crates/raikiri-layout-style" }
raikiri-style = { path = "$tree/crates/raikiri-style" }
raikiri-traits = { path = "$tree/crates/raikiri-traits" }
taffy = { version = ">=0.15", default-features = false, features = ["std", "flexbox", "grid", "block_layout", "content_size", "calc", "float_layout", "detailed_layout_info"] }
url = "2.5"

[profile.release]
debug-assertions = true
overflow-checks = true

[workspace]
EOF
}

# Builds the tool's sources and build script from the current tree against
# the crates of tree $1, with that tree's lockfile, into a target directory
# owned by that tree, then copies the binary to $2. A target directory per
# tree keeps concurrent sessions from overwriting each other's binaries.
build_tool() {
  local tree="$1" binary="$2"
  local manifest_dir
  manifest_dir="$(mktemp -d "$SCRATCH/tool.XXXXXX")"
  cp -rf "$REPO_ROOT/crates/raikiri-layout-diff/src" "$manifest_dir/src"
  cp -f "$REPO_ROOT/crates/raikiri-layout-diff/build.rs" "$manifest_dir/build.rs"
  tool_manifest "$tree" > "$manifest_dir/Cargo.toml"
  cp -f "$tree/Cargo.lock" "$manifest_dir/Cargo.lock"
  CARGO_TARGET_DIR="$tree/target/layout-diff" \
    cargo build --release --quiet --manifest-path "$manifest_dir/Cargo.toml"
  cp -f "$tree/target/layout-diff/release/raikiri-layout-diff" "$binary"
}

# Prints the build settings cargo takes from outside the tree: the
# environment variables that change how it compiles, and the cargo
# configuration files that apply where it runs, in this checkout and the
# directories above it, and in CARGO_HOME. Both builds run with them, so a
# cached build made with others is not reused.
build_settings() {
  local name dir file
  while IFS= read -r name; do
    printf '%s=%s\n' "$name" "${!name}"
  done < <(compgen -e | LC_ALL=C sort \
    | grep -E '^(RUSTFLAGS|RUSTDOCFLAGS|RUSTC|RUSTC_[A-Z0-9_]+|CARGO_ENCODED_RUSTFLAGS|CARGO_BUILD_[A-Z0-9_]+|CARGO_PROFILE_[A-Z0-9_]+|CARGO_TARGET_[A-Z0-9_]+|CARGO_UNSTABLE_[A-Z0-9_]+)$' \
    | grep -v -x CARGO_TARGET_DIR || true)
  dir="$REPO_ROOT"
  while :; do
    for file in "$dir/.cargo/config.toml" "$dir/.cargo/config"; do
      if [[ -f "$file" ]]; then
        echo "== $file"
        cat "$file"
      fi
    done
    [[ "$dir" == / ]] && break
    dir="$(dirname "$dir")"
  done
  for file in "${CARGO_HOME:-$HOME/.cargo}/config.toml" "${CARGO_HOME:-$HOME/.cargo}/config"; do
    if [[ -f "$file" ]]; then
      echo "== $file"
      cat "$file"
    fi
  done
}

# What the merge-base build depends on besides the tree's files, which the
# commit already names: the tool's sources and manifest, the compiler, given
# as $1, and the build settings.
base_build_key() {
  {
    echo "$BASE_SHA"
    (cd "$REPO_ROOT/crates/raikiri-layout-diff" && find build.rs src -type f -print0 | LC_ALL=C sort -z | xargs -0 sha256sum)
    tool_manifest TREE
    printf '%s\n' "$1"
    build_settings
  } | sha256sum | cut -d' ' -f1
}

# Prints the names of the environment variables the release build in target
# directory $1 declared that it reads: those its build scripts asked cargo to
# watch (`cargo:rerun-if-env-changed`), and those its crates read at compile
# time, which rustc records in their dep-info files.
declared_environment() {
  local release="$1/release"
  {
    cat "$release"/build/*/output 2>/dev/null \
      | sed -n -E 's/^cargo::?rerun-if-env-changed=(.*)$/\1/p' || true
    cat "$release"/deps/*.d 2>/dev/null \
      | sed -n -E 's/^# env-dep:([^=]*).*$/\1/p' || true
  } | LC_ALL=C sort -u
}

# Prints the value of each environment variable named on standard input, or
# that it is unset. A value is printed as its bytes in hex, so that none of
# them, trailing newlines included, is lost on the way.
environment_values() {
  local name
  while IFS= read -r name; do
    if printenv -- "$name" > /dev/null; then
      printf '%s=%s\n' "$name" "$(printenv -- "$name" | od -A n -v -t x1 | tr -d ' \n')"
    else
      printf '%s is unset\n' "$name"
    fi
  done
}

# Copies the cached merge-base build to $1, or builds it in a throwaway
# worktree, caches it and copies it. $2 is the head build.
#
# The compiler is the one cargo built the head with, which the head reports:
# cargo picks it from RUSTC, build.rustc or the default, and gives it to the
# tool's build script, which records its version. A compiler that cargo
# sees change rebuilds the head, so the key changes with it. A cached build
# is reused only when that compiler built it too, and only while the
# environment variables it declared that it reads keep the values they had
# when it was built, which is what cargo checks before reusing a build. Each
# entry is named by the build key and a hash of those values, is published
# whole by renaming a finished directory into place, and is never changed
# after, so a run sees a complete entry or none.
base_build() {
  local binary="$1" head="$2" compiler key entry partial
  compiler="$("$head" compiler)"
  key="$(base_build_key "$compiler")"
  if [[ "$USE_CACHE" -eq 1 ]]; then
    for entry in "$CACHE_ROOT/$key"-*/; do
      entry="${entry%/}"
      [[ -f "$entry/variables" && -x "$entry/raikiri-layout-diff" ]] || continue
      [[ "$(environment_values < "$entry/variables" | sha256sum | cut -c1-16)" == "${entry##*-}" ]] \
        || continue
      [[ "$("$entry/raikiri-layout-diff" compiler 2>/dev/null)" == "$compiler" ]] || continue
      # Another run may remove the entry meanwhile. Before the copy, this
      # one then builds. After it, marking the entry as used must not make
      # it again, as a file that no later build could be renamed over.
      if cp -f "$entry/raikiri-layout-diff" "$binary" 2>/dev/null; then
        echo "-- using the cached build of merge-base $BASE_SHA ($entry) --"
        touch -c "$entry"
        return 0
      fi
    done
  fi
  echo "-- checking out merge-base $BASE_SHA into a detached worktree --"
  mkdir -p "$MAIN_ROOT/.worktrees"
  # Repository hooks are for working checkouts, not for this throwaway tree.
  git -c core.hooksPath=/dev/null worktree add --detach "$BASE_TREE" "$BASE_SHA" >/dev/null
  echo "-- building base ($BASE_TREE) --"
  build_tool "$BASE_TREE" "$binary"
  partial="$CACHE_ROOT/.partial-$key-$$"
  mkdir -p "$partial"
  declared_environment "$BASE_TREE/target/layout-diff" > "$partial/variables"
  environment_values < "$partial/variables" > "$partial/environment"
  cp -f "$binary" "$partial/raikiri-layout-diff"
  entry="$CACHE_ROOT/$key-$(sha256sum < "$partial/environment" | cut -c1-16)"
  # When another run has published the same entry, the rename fails, even
  # when forced, and that one is kept.
  mv -f -T "$partial" "$entry" 2>/dev/null || rm -rf "$partial"
  local stale
  # Entries only: a build another run is still publishing is not one yet, and
  # is removed below once it is clearly abandoned.
  find "$CACHE_ROOT" -mindepth 1 -maxdepth 1 -type d ! -name '.partial-*' -printf '%T@ %p\n' \
    | sort -rn | tail -n +"$((CACHE_KEEP + 1))" | cut -d' ' -f2- \
    | while IFS= read -r stale; do rm -rf "$stale"; done
  find "$CACHE_ROOT" -mindepth 1 -maxdepth 1 -type d -name '.partial-*' -mmin +1440 \
    -exec rm -rf {} + 2>/dev/null || true
}

echo "-- building head ($REPO_ROOT) --"
build_tool "$REPO_ROOT" "$SCRATCH/diff-head"
base_build "$SCRATCH/diff-base" "$SCRATCH/diff-head"
echo

# Both builds come from the same tool sources, so they must generate the same
# inputs; a mismatch means a stale build, not a layout change.
for seed in 0 1 2 3 4 5 6 7; do
  "$SCRATCH/diff-base" describe "$seed" > "$SCRATCH/describe-base.txt"
  "$SCRATCH/diff-head" describe "$seed" > "$SCRATCH/describe-head.txt"
  if ! cmp -s "$SCRATCH/describe-base.txt" "$SCRATCH/describe-head.txt"; then
    echo "error: the two builds generate different inputs for seed $seed" >&2
    exit 2
  fi
done

# The revision of the shared WPT checkout itself, never of a repository that
# happens to enclose it; empty when it cannot be read.
wpt_revision() {
  git --git-dir="$WPT_ROOT/.git" rev-parse HEAD 2>/dev/null || true
}

DUMP_ARGS=(dump --font "$FONT" --seeds "0..$SEEDS")
SHOW_ARGS=(show --font "$FONT")
WPT_REVISION=""
if [[ "$WPT_BASELINE" -eq 1 || ${#WPT_DIRS[@]} -gt 0 ]]; then
  WPT_ROOT="$(wpt_cache_dir)"
  WPT_REVISION="$(wpt_revision)"
  if [[ -z "$WPT_REVISION" ]]; then
    echo "error: cannot read the revision of the WPT checkout at $WPT_ROOT (run scripts/wpt/fetch.sh first)" >&2
    exit 2
  fi
  HTML_LIST="$SCRATCH/html-cases.txt"
  : > "$HTML_LIST"
  if [[ "$WPT_BASELINE" -eq 1 ]]; then
    # The baseline and the expected failures name tests by root-relative path.
    # A test with several variants (`?query`) is one file, and a `.xhtml`
    # test is an XML document, which the WPT runner parses as XML; this
    # tool lays out every document through the HTML parser, which reads it
    # as something else, so those are left out.
    {
      grep -v -e '^#' -e '^[[:space:]]*$' "$REPO_ROOT/expectations/raikiri-baseline.txt" || true
      awk -F' [|] ' '!/^#/ && NF { print $1 }' "$REPO_ROOT/expectations/expected-failures.txt"
    } | sed -e 's/?.*$//' | { grep -v '\.xhtml$' || true; } | LC_ALL=C sort -u > "$SCRATCH/baseline-ids.txt"
    while IFS= read -r id; do
      if [[ ! -f "$WPT_ROOT/$id" ]]; then
        echo "error: $WPT_ROOT/$id does not exist (run scripts/wpt/fetch.sh first)" >&2
        exit 2
      fi
      printf '%s\n' "$WPT_ROOT/$id" >> "$HTML_LIST"
    done < "$SCRATCH/baseline-ids.txt"
  fi
  for dir in ${WPT_DIRS[@]+"${WPT_DIRS[@]}"}; do
    if [[ ! -d "$WPT_ROOT/$dir" ]]; then
      echo "error: $WPT_ROOT/$dir does not exist (run scripts/wpt/fetch.sh first)" >&2
      exit 2
    fi
    find "$WPT_ROOT/$dir" -type f -name '*.html' | LC_ALL=C sort >> "$HTML_LIST"
  done
  # A file named twice is one case.
  awk '!seen[$0]++' "$HTML_LIST" > "$HTML_LIST.unique"
  mv -f "$HTML_LIST.unique" "$HTML_LIST"
  echo "html cases: $(wc -l < "$HTML_LIST") files at WPT $WPT_REVISION"
  DUMP_ARGS+=(--html-list "$HTML_LIST" --html-root "$WPT_ROOT")
  SHOW_ARGS+=(--html-root "$WPT_ROOT")
fi

# Starts build $1 on every case in the background, writing to $2. The two
# builds run side by side: they read the same files and share nothing else.
start_dump() {
  local binary="$1" output="$2"
  # `--foreground` keeps the tool in the terminal's process group, so that
  # the run ends with it.
  # The engine's diagnostics on standard error are noise for a comparison;
  # they are shown when the build fails.
  timeout --foreground "$TIMEOUT" "$binary" "${DUMP_ARGS[@]}" > "$output" 2> "$output.stderr" &
  DUMP_PIDS+=("$!")
}

# Waits for the dump of build $1 that process $3 runs into $2. A crash or a
# hang is a tooling error: the tool writes each case id before running the
# case, so an unfinished last line names the case it stopped in.
finish_dump() {
  local label="$1" output="$2" pid="$3" status=0
  wait "$pid" || status=$?
  if [[ $status -eq 0 ]]; then
    return 0
  fi
  if [[ $status -eq 124 ]]; then
    echo "error: the $label build did not finish within ${TIMEOUT}s" >&2
  else
    echo "error: the $label build exited with status $status" >&2
  fi
  if [[ -s "$output" && -n "$(tail -c 1 "$output")" ]]; then
    echo "while running case: $(tail -n 1 "$output" | cut -f1)" >&2
  fi
  tail -n 5 "$output.stderr" >&2
  exit 2
}

echo "-- dumping base and head --"
start_dump "$SCRATCH/diff-base" "$SCRATCH/base.tsv"
start_dump "$SCRATCH/diff-head" "$SCRATCH/head.tsv"
finish_dump base "$SCRATCH/base.tsv" "${DUMP_PIDS[0]}"
finish_dump head "$SCRATCH/head.tsv" "${DUMP_PIDS[1]}"
echo

# The checkout is shared between sessions: an update during the run would let
# the two builds read different files under the same case ids.
if [[ -n "$WPT_REVISION" ]]; then
  revision_now="$(wpt_revision)"
  if [[ "$revision_now" != "$WPT_REVISION" ]]; then
    echo "error: the WPT checkout moved from $WPT_REVISION to $revision_now during the run" >&2
    exit 2
  fi
fi

# Columns: case, hash, status (ok | error | panic), panic message. The awk
# status is 4 when no case has layout output from both builds.
compare_status=0
awk -F'\t' -v differing="$SCRATCH/differing.txt" '
  NR == FNR { hash[$1] = $2; status[$1] = $3; next }
  {
    total++
    if ($3 == "ok" && status[$1] == "ok") { compared++ }
    if (!($1 in hash)) { missing++; print $1 > differing; next }
    if (hash[$1] != $2) { changed++; print $1 > differing }
    if ($3 == "panic" && status[$1] != "panic") { new_panics++; print "new panic: " $1 ": " $4 }
    if ($3 != "panic" && status[$1] == "panic") { fixed_panics++ }
    if ($3 == "panic") { panics++ }
    if ($3 == "error") { errors++ }
  }
  END {
    printf "cases      : %d (%d with layout output from both builds)\n", total, compared
    printf "differing  : %d\n", changed + missing
    printf "new panics : %d\n", new_panics
    printf "panics fixed by head: %d\n", fixed_panics
    printf "panics in head      : %d\n", panics
    printf "errors in head      : %d\n", errors
    if (compared == 0) { exit 4 }
  }
' "$SCRATCH/base.tsv" "$SCRATCH/head.tsv" || compare_status=$?

if [[ ! -s "$SCRATCH/differing.txt" ]]; then
  if [[ $compare_status -ne 0 ]]; then
    echo >&2
    echo "error: no case has layout output from both builds, so nothing was compared" >&2
    exit 2
  fi
  echo
  echo "PASS: every case has the same canonical layout output"
  exit 0
fi

echo
echo "first differing cases:"
head -n 20 "$SCRATCH/differing.txt"
shown=0
while IFS= read -r case_id && [[ $shown -lt $SHOW ]]; do
  shown=$((shown + 1))
  echo
  echo "== diff for $case_id (base → head) =="
  diff -u \
    <("$SCRATCH/diff-base" "${SHOW_ARGS[@]}" "$case_id") \
    <("$SCRATCH/diff-head" "${SHOW_ARGS[@]}" "$case_id") | head -n 60 || true
done < "$SCRATCH/differing.txt"
echo
echo "FAIL: some cases differ between the merge-base and HEAD"
exit 1
