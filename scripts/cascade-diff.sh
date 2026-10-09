#!/usr/bin/env bash
# Compare cascade results between the merge-base and HEAD.
#
# Usage: scripts/cascade-diff.sh [--base <ref>] [--seeds <n>] [--wpt <dir>]...
#                                [--wpt-default] [--show <n>] [--timeout <s>]
#
#   --base <ref>    Ref whose merge-base with HEAD is the baseline
#                   (default: origin/main).
#   --seeds <n>     Generated cases use seeds 0..n (default: 2000). Each seed is
#                   cascaded for print, for screen, and through the
#                   ::first-line entry point.
#   --wpt <dir>     Add every .html file under this directory of the WPT
#                   checkout (for example css/css-cascade) as a case. Repeatable.
#   --wpt-default   Add the default set of CSS directories listed below.
#   --show <n>      Print a field-level diff for the first n differing cases
#                   (default: 3).
#   --timeout <s>   Stop a build that has not run every case after this many
#                   seconds (default: 3600).
#
# Builds crates/raikiri-cascade-diff twice — against a detached checkout of
# the merge-base and against the current tree — runs both builds on the same
# cases, and reports every case whose canonical cascade output differs. A
# change that is meant to preserve cascade behavior must report none. Cases
# that panic are compared like any other output, so a new panic is reported as
# a difference. Both builds enable debug assertions and overflow checks, so a
# broken internal invariant shows up as a panic as well.
#
# Exit status: 0 when no case differs, 1 when some case differs, 2 on a usage
# or tooling error, including a build that crashes or hangs on a case (the
# case is named).

set -Eeuo pipefail
trap 'echo "cascade-diff.sh: line $LINENO: \`$BASH_COMMAND\` failed" >&2; exit 2' ERR

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
WPT_DIRS=()
DEFAULT_WPT_DIRS=(
  css/css-cascade css/css-variables css/selectors css/css-pseudo css/css-lists
  css/css-tables css/css-text css/css-backgrounds css/css-box css/css-page
  css/css-content css/css-writing-modes css/css-fonts css/css-display
)

usage_error() {
  echo "cascade-diff.sh: $1" >&2
  exit 2
}

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base) [[ $# -ge 2 ]] || usage_error "--base needs a ref"; BASE_REF="$2"; shift 2 ;;
    --seeds) [[ $# -ge 2 ]] || usage_error "--seeds needs a count"; SEEDS="$2"; shift 2 ;;
    --wpt) [[ $# -ge 2 ]] || usage_error "--wpt needs a directory"; WPT_DIRS+=("$2"); shift 2 ;;
    --wpt-default) WPT_DIRS+=("${DEFAULT_WPT_DIRS[@]}"); shift ;;
    --show) [[ $# -ge 2 ]] || usage_error "--show needs a count"; SHOW="$2"; shift 2 ;;
    --timeout) [[ $# -ge 2 ]] || usage_error "--timeout needs seconds"; TIMEOUT="$2"; shift 2 ;;
    *) usage_error "unknown argument: $1" ;;
  esac
done
[[ "$SEEDS" =~ ^[0-9]{1,7}$ ]] || usage_error "--seeds must be a non-negative integer"
[[ "$SHOW" =~ ^[0-9]{1,4}$ ]] || usage_error "--show must be a non-negative integer"
[[ "$TIMEOUT" =~ ^[1-9][0-9]{0,6}$ ]] || usage_error "--timeout must be a positive integer"

BASE_SHA="$(git merge-base "$BASE_REF" HEAD)"
SCRATCH="$(mktemp -d "$TMPDIR/raikiri-cascade-diff.XXXXXX")"
BASE_TREE="$SCRATCH/base-tree"

cleanup() {
  git -C "$REPO_ROOT" worktree remove --force "$BASE_TREE" >/dev/null 2>&1 || true
  rm -rf "$SCRATCH"
}
trap cleanup EXIT

echo "== scripts/cascade-diff.sh =="
echo "head      : $(git rev-parse HEAD)"
echo "base ref  : $BASE_REF"
echo "base sha  : $BASE_SHA"
echo "seeds     : 0..$SEEDS"
echo "TMPDIR    : $TMPDIR"
echo

# Builds the tool's sources from the current tree against the raikiri-style and
# raikiri-html crates of tree $1, with that tree's lockfile, into a target
# directory owned by that tree, then copies the binary to $2. A target
# directory per tree keeps concurrent sessions from overwriting each other's
# binaries.
build_tool() {
  local tree="$1" binary="$2"
  local manifest_dir
  manifest_dir="$(mktemp -d "$SCRATCH/tool.XXXXXX")"
  cp -rf "$REPO_ROOT/crates/raikiri-cascade-diff/src" "$manifest_dir/src"
  cat > "$manifest_dir/Cargo.toml" <<EOF
[package]
name = "raikiri-cascade-diff"
version = "0.0.0"
edition = "2024"
publish = false

[dependencies]
raikiri-style = { path = "$tree/crates/raikiri-style" }
raikiri-html = { path = "$tree/crates/raikiri-html" }

[profile.release]
debug-assertions = true
overflow-checks = true

[workspace]
EOF
  cp -f "$tree/Cargo.lock" "$manifest_dir/Cargo.lock"
  CARGO_TARGET_DIR="$tree/target/cascade-diff" \
    cargo build --release --quiet --manifest-path "$manifest_dir/Cargo.toml"
  cp -f "$tree/target/cascade-diff/release/raikiri-cascade-diff" "$binary"
}

echo "-- building head ($REPO_ROOT) --"
build_tool "$REPO_ROOT" "$SCRATCH/diff-head"
echo "-- checking out merge-base $BASE_SHA into a detached worktree --"
# Repository hooks are for working checkouts, not for this throwaway tree.
git -c core.hooksPath=/dev/null worktree add --detach "$BASE_TREE" "$BASE_SHA" >/dev/null
echo "-- building base ($BASE_TREE) --"
build_tool "$BASE_TREE" "$SCRATCH/diff-base"
echo

# Both builds come from the same tool sources, so they must generate the same
# inputs; a mismatch means a stale build, not a cascade change.
for seed in 0 1 2 3 4 5 6 7; do
  if ! cmp -s <("$SCRATCH/diff-base" describe "$seed") <("$SCRATCH/diff-head" describe "$seed"); then
    echo "error: the two builds generate different inputs for seed $seed" >&2
    exit 2
  fi
done

DUMP_ARGS=(dump --seeds "0..$SEEDS")
WPT_REVISION=""
if [[ ${#WPT_DIRS[@]} -gt 0 ]]; then
  WPT_ROOT="$(wpt_cache_dir)"
  WPT_REVISION="$(git -C "$WPT_ROOT" rev-parse HEAD 2>/dev/null || echo unknown)"
  HTML_LIST="$SCRATCH/html-cases.txt"
  : > "$HTML_LIST"
  for dir in "${WPT_DIRS[@]}"; do
    if [[ ! -d "$WPT_ROOT/$dir" ]]; then
      echo "error: $WPT_ROOT/$dir does not exist (run scripts/wpt/fetch.sh first)" >&2
      exit 2
    fi
    find "$WPT_ROOT/$dir" -type f -name '*.html' | LC_ALL=C sort >> "$HTML_LIST"
  done
  echo "html cases: $(wc -l < "$HTML_LIST") files from ${#WPT_DIRS[@]} WPT directories at $WPT_REVISION"
  DUMP_ARGS+=(--html-list "$HTML_LIST")
fi

# Runs build $2 on every case into $3. A crash or a hang is a tooling error:
# the tool writes each case id before running the case, so an unfinished last
# line names the case it stopped in.
run_dump() {
  local label="$1" binary="$2" output="$3" status=0
  timeout "$TIMEOUT" "$binary" "${DUMP_ARGS[@]}" > "$output" || status=$?
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
  exit 2
}

echo "-- dumping base --"
run_dump base "$SCRATCH/diff-base" "$SCRATCH/base.tsv"
echo "-- dumping head --"
run_dump head "$SCRATCH/diff-head" "$SCRATCH/head.tsv"
echo

# The checkout is shared between sessions: an update during the run would let
# the two builds read different files under the same case ids.
if [[ -n "$WPT_REVISION" ]]; then
  revision_now="$(git -C "$WPT_ROOT" rev-parse HEAD 2>/dev/null || echo unknown)"
  if [[ "$revision_now" != "$WPT_REVISION" ]]; then
    echo "error: the WPT checkout moved from $WPT_REVISION to $revision_now during the run" >&2
    exit 2
  fi
fi

# Columns: case, hash, status (ok | error | panic), panic message.
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
    printf "cases      : %d (%d with cascade output from both builds)\n", total, compared
    printf "differing  : %d\n", changed + missing
    printf "new panics : %d\n", new_panics
    printf "panics fixed by head: %d\n", fixed_panics
    printf "panics in head      : %d\n", panics
    printf "errors in head      : %d\n", errors
  }
' "$SCRATCH/base.tsv" "$SCRATCH/head.tsv"

if [[ ! -s "$SCRATCH/differing.txt" ]]; then
  echo
  echo "PASS: every case has the same canonical cascade output"
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
    <("$SCRATCH/diff-base" show "$case_id") \
    <("$SCRATCH/diff-head" show "$case_id") | head -n 60 || true
done < "$SCRATCH/differing.txt"
echo
echo "FAIL: some cases differ between the merge-base and HEAD"
exit 1
