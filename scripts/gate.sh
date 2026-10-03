#!/usr/bin/env bash
# Run the repository's automated validation checks in one command.
#
# Usage: scripts/gate.sh [--base <ref>] [--skip-coverage] [--with-bench]
#
#   --base <ref>       Base ref for source-change applicability and patch
#                       coverage (default: main).
#   --skip-coverage    Skip patch coverage for a fast test and lint iteration.
#   --with-bench       Also run the optional cascade benchmark comparison.
#
# Exit status is 0 when every applicable check passes. The output records the
# commands and their results for the caller to inspect.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Resolve REPO_ROOT from the caller's shell cwd (not from this script's own
# on-disk location) and hard-fail if the invoked file and the cwd disagree
# on which git tree is meant — see scripts/lib/repo_root.sh for the full
# rationale. This script delegates to sibling scripts as
# "$SCRIPT_DIR/<name>.sh" (patch-coverage.sh, cascade-bench-compare.sh
# below), which source the same helper, so the guard applies consistently
# one level down too.
# shellcheck source=lib/repo_root.sh
source "$SCRIPT_DIR/lib/repo_root.sh"
# shellcheck source=wpt/lib/cache_path.sh
source "$SCRIPT_DIR/wpt/lib/cache_path.sh"

cd "$REPO_ROOT"

# shellcheck source=lib/tmpdir.sh
source "$SCRIPT_DIR/lib/tmpdir.sh"

BASE_REF="main"
SKIP_COVERAGE=0
WITH_BENCH=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --base)
      BASE_REF="$2"
      shift 2
      ;;
    --skip-coverage)
      SKIP_COVERAGE=1
      shift
      ;;
    --with-bench)
      WITH_BENCH=1
      shift
      ;;
    -h|--help)
      awk 'NR==1 { next } /^#/ { print; next } { exit }' "${BASH_SOURCE[0]}"
      exit 0
      ;;
    *)
      echo "gate.sh: unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

echo "== scripts/gate.sh =="
echo "repo root : $REPO_ROOT"
echo "base ref  : $BASE_REF"
echo "TMPDIR    : $TMPDIR"
echo

FAIL=0

echo "-- safe_merge evidence tests --"
if ! bash "$SCRIPT_DIR/test-safe-merge.sh"; then
  echo "FAIL: safe_merge evidence checks failed"
  FAIL=1
fi
echo

# ── source-change applicability test (mechanical) ───────────────────────────
# Skip the main validation run only when the name-only diff has no paths that
# indicate Rust or other compiled source inputs. Renames are expanded by
# --no-renames. Gitlinks and symlink retargets still need caller inspection.
echo "-- source-change applicability test --"
APPLICABILITY_DIFF="$(git diff --no-renames --name-only "$BASE_REF"...HEAD)"
echo "git diff --no-renames --name-only $BASE_REF...HEAD"
if [[ -z "$APPLICABILITY_DIFF" ]]; then
  echo "(empty diff)"
else
  echo "$APPLICABILITY_DIFF"
fi

if echo "$APPLICABILITY_DIFF" | grep -qE '\.rs$|\.css$|(^|/)Cargo\.(toml|lock)$|(^|/)expectations/.*\.txt$'; then
  APPLICABLE=1
  echo "=> matching source input found; validation applies"
else
  APPLICABLE=0
  echo "=> no match on file names alone. The remaining path-name"
  echo "   applicability limits: renames' source side is handled by"
  echo "   --no-renames above; gitlinks and symlink retargets are not checked"
  echo "   here. Review those changed paths before treating this as a skip."
fi
echo

# The optional benchmark comparison runs before the applicability early exit,
# so an explicit --with-bench request is always honored. It is useful for
# changes near the cascade hot loop and is not part of the default run.
if [[ "$WITH_BENCH" -eq 1 ]]; then
  if [[ -x "$SCRIPT_DIR/cascade-bench-compare.sh" ]]; then
    echo "-- optional: cascade benchmark comparison --"
    if ! "$SCRIPT_DIR/cascade-bench-compare.sh" "$BASE_REF"; then
      echo "FAIL: cascade-bench-compare.sh reported a regression past threshold"
      FAIL=1
    fi
  else
    echo "-- optional: --with-bench passed but scripts/cascade-bench-compare.sh"
    echo "   not found (benchmark comparison script is not present in this tree) --"
    FAIL=1
  fi
  echo
fi

if [[ "$APPLICABLE" -eq 0 ]]; then
  echo "Tests, static checks, and patch coverage are skipped for this diff."
  echo "Record this output with your own inspection of the changed paths."
  echo "This run did not execute the main validation checks."
  if [[ "$WITH_BENCH" -eq 1 ]]; then
    echo "note: --with-bench was requested and ran above; source applicability"
    echo "      does not affect the optional benchmark result."
  fi
  echo
  echo "== gate.sh summary =="
  if [[ "$FAIL" -eq 0 ]]; then
    echo "PASS"
    exit 0
  else
    echo "FAIL"
    exit 1
  fi
fi

# ── Test execution ──────────────────────────────────────────────────────────
echo "-- cargo test --workspace --locked --"
if ! cargo test --workspace --locked; then
  echo "FAIL: cargo test --workspace --locked"
  FAIL=1
fi
echo

# Non-default crate features that no workspace member enables are never
# compiled by the --workspace runs above/below, so each one is exercised
# explicitly here (and in the clippy/doc sections). Keep this list in sync
# with .github/workflows/ci.yml.
echo "-- cargo test -p raikiri-net --features http-ureq --locked --"
if ! cargo test -p raikiri-net --features http-ureq --locked; then
  echo "FAIL: cargo test -p raikiri-net --features http-ureq --locked"
  FAIL=1
fi
echo

echo "-- #[ignore] scan --"
CENSUS_CMD="git grep -nE '#\[ignore(\]| *=)' -- '*.rs'"
echo "$CENSUS_CMD"
set +e
CENSUS_OUTPUT="$(git grep -nE '#\[ignore(\]| *=)' -- '*.rs')"
CENSUS_STATUS=$?
set -e
if [[ -z "$CENSUS_OUTPUT" ]]; then
  echo "(0 matches)"
else
  echo "$CENSUS_OUTPUT"
fi
CENSUS_COUNT=$(echo "$CENSUS_OUTPUT" | grep -c . || true)
if [[ "$CENSUS_STATUS" -ne 0 ]]; then
  CENSUS_COUNT=0
fi
echo "scan matches: $CENSUS_COUNT"
echo

if [[ "$CENSUS_COUNT" -eq 0 ]]; then
  echo "-- --ignored run: skipped (scan found no ignored tests) --"
  IGNORED_N=0
else
  # The ignored WPT tests need target/wpt (VRT font fixtures) fetched.
  # target/ is per-worktree build output and may have been removed by
  # cargo clean; recreate the compatibility link directly from the persistent
  # home cache instead of depending on the main worktree's target/ directory.
  if [[ ! -e "$REPO_ROOT/target/wpt" ]]; then
    WPT_CACHE_DIR="$(wpt_cache_dir)"
    if [[ -e "$WPT_CACHE_DIR/.git" ]]; then
      ensure_wpt_cache_link "$REPO_ROOT" "$WPT_CACHE_DIR"
      echo "(re-linked target/wpt -> $WPT_CACHE_DIR)"
    else
      echo "warning: WPT cache missing at $WPT_CACHE_DIR. Run"
      echo "         scripts/wpt/fetch.sh before the --ignored run if it fails"
      echo "         on VRT font fixtures."
    fi
  fi

  echo "-- cargo test --workspace --locked -- --ignored --"
  echo "cargo test --workspace --locked -- --ignored"
  set +e
  IGNORED_OUTPUT="$(cargo test --workspace --locked -- --ignored 2>&1)"
  IGNORED_STATUS=$?
  set -e
  echo "$IGNORED_OUTPUT"
  if [[ "$IGNORED_STATUS" -ne 0 ]]; then
    echo "FAIL: cargo test --workspace --locked -- --ignored"
    FAIL=1
  fi
  # Sum "N passed" across every test runner's "test result:" summary line —
  # validation run record: this is the sum the record must cite, not any single
  # test runner's line and not the normal run's "N ignored" figure.
  IGNORED_N=$(echo "$IGNORED_OUTPUT" | grep -oE '[0-9]+ passed' | grep -oE '[0-9]+' | awk '{s+=$1} END {print s+0}')
  echo "N passed (summed across all test-runner summary lines): $IGNORED_N"
  if [[ "$IGNORED_N" -eq 0 ]]; then
    echo "FAIL: N passed == 0 on a non-zero scan; the ignored tests did not run"
    FAIL=1
  fi
fi
echo

# ── Static checks ───────────────────────────────────────────────────────────
echo "-- cargo fmt --all --check --"
if ! cargo fmt --all --check; then
  echo "FAIL: cargo fmt --all --check"
  FAIL=1
fi
echo

echo "-- scripts/orphan-tests-lint.sh --"
# Catches a tests.rs split (AGENTS.md "move unit tests to tests.rs") that
# forgot its `mod tests;` declaration — rustc silently never compiles such a
# file (no warning, no error) and coverage stays green because the file was
# never in the coverage report to begin with. See
# scripts/lib/orphan_tests_check.py's module docstring.
if ! "$SCRIPT_DIR/orphan-tests-lint.sh"; then
  echo "FAIL: scripts/orphan-tests-lint.sh found an orphan tests.rs/*_tests.rs file"
  FAIL=1
fi
echo

echo "-- cargo clippy --workspace --all-targets -- -D warnings --"
if ! cargo clippy --workspace --all-targets --locked -- -D warnings; then
  echo "FAIL: cargo clippy --workspace --all-targets --locked -- -D warnings"
  FAIL=1
fi
echo

echo "-- cargo clippy -p raikiri-net --features http-ureq --all-targets -- -D warnings --"
if ! cargo clippy -p raikiri-net --features http-ureq --all-targets --locked -- -D warnings; then
  echo "FAIL: cargo clippy -p raikiri-net --features http-ureq --all-targets --locked -- -D warnings"
  FAIL=1
fi
echo

echo "-- RUSTDOCFLAGS=\"-D warnings\" cargo doc --no-deps --workspace --document-private-items --"
# --document-private-items is required here: without it, rustdoc only
# resolves intra-doc links inside public items, so a broken link inside a
# private (non-pub) item's doc comment is silently skipped instead of
# hard-erroring under -D warnings.
if ! RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --document-private-items --locked; then
  echo "FAIL: cargo doc --no-deps --workspace --document-private-items"
  FAIL=1
fi
echo

echo "-- RUSTDOCFLAGS=\"-D warnings\" cargo doc --no-deps -p raikiri-net --features http-ureq --document-private-items --"
if ! RUSTDOCFLAGS="-D warnings" cargo doc --no-deps -p raikiri-net --features http-ureq --document-private-items --locked; then
  echo "FAIL: cargo doc --no-deps -p raikiri-net --features http-ureq --document-private-items"
  FAIL=1
fi
echo

# NB: The expectations/*.txt lint (WPT status file consistency) has
# no dedicated script in this repo yet (the raikiri-wpt validate-expectations
# binary covers a related but not identical surface; see
# crates/raikiri-wpt/src/bin/validate-expectations.rs). Keep this gap visible
# instead of silently omitting it.
echo "-- expectations/*.txt lint --"
echo "NOT AUTOMATED by this script yet (no dedicated lint entry point found)."
echo "See crates/raikiri-wpt/src/bin/validate-expectations.rs for the closest"
echo "existing tool and confirm manually until this is connected."
echo

# ── Patch coverage ──────────────────────────────────────────────────────────
if [[ "$SKIP_COVERAGE" -eq 1 ]]; then
  echo "-- patch coverage: --skip-coverage passed, skipping --"
elif [[ -x "$SCRIPT_DIR/patch-coverage.sh" ]]; then
  echo "-- patch coverage (scripts/patch-coverage.sh --base $BASE_REF) --"
  set +e
  "$SCRIPT_DIR/patch-coverage.sh" "$BASE_REF"
  PATCH_COVERAGE_STATUS=$?
  set -e
  if [[ "$PATCH_COVERAGE_STATUS" -eq 1 ]]; then
    echo "FAIL: scripts/patch-coverage.sh reported uncovered changed lines"
    echo "      without a cov:ignore escape. Add a"
    echo "      covering test (in-scope) or escalate to a follow-up item"
    echo "      (out-of-scope), then re-run."
    FAIL=1
  elif [[ "$PATCH_COVERAGE_STATUS" -eq 2 ]]; then
    echo "FAIL: scripts/patch-coverage.sh's coverage measurement could not"
    echo "      complete (exit 2: infra failure, not an uncovered-line"
    echo "      finding) — a dirty tree (uncommitted *.rs changes),"
    echo "      cargo-llvm-cov not installed, or a cargo metadata failure."
    echo "      See patch-coverage.sh's own stderr output for which one."
    echo "      Fix that infra problem and re-run; adding a test will not"
    echo "      fix this."
    FAIL=1
  elif [[ "$PATCH_COVERAGE_STATUS" -ne 0 ]]; then
    echo "FAIL: scripts/patch-coverage.sh exited $PATCH_COVERAGE_STATUS, which"
    echo "      is outside its documented 0/1/2 contract (see its own"
    echo "      \"Exit status\" header comment). Treat this as unverified"
    echo "      rather than either FAIL message above — investigate the"
    echo "      script's output before re-running."
    FAIL=1
  fi
else
  echo "-- patch coverage: scripts/patch-coverage.sh not found --"
  echo "   (patch-coverage.sh is not present in this tree). Patch"
  echo "   coverage was NOT measured by this run — do not record this as a"
  echo "   pass."
fi
echo

echo "== gate.sh summary =="
if [[ "$FAIL" -eq 0 ]]; then
  echo "PASS"
  exit 0
else
  echo "FAIL"
  exit 1
fi
