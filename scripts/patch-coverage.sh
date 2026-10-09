#!/usr/bin/env bash
# scripts/patch-coverage.sh — patch coverage, measured.
#
# The gate requires every changed line to be covered or escalated. Before
# this script existed nothing in this repo could measure that, so it was
# checked by manual listing and reviewer spot-checks.
#
# Approach (workspace lcov → merge-base diff → gated-crate judgment,
# modeled on flpdf's patch-coverage.sh):
#
#   1. `cargo llvm-cov clean --workspace`, then run the *whole workspace*
#      test suite under cargo-llvm-cov instrumentation, export lcov (the
#      clean is required: incremental runs report stale line tables).
#   2. Diff merge-base(<base>, HEAD)..HEAD for *.rs, in -U0 form, to get the
#      exact added-line set per file. The base is resolved once via
#      `git merge-base`, not a bare ref, so a concurrent merge on <base>
#      after this branch forked cannot change what counts as "this diff's
#      lines".
#   3. For each added line: covered (lcov hit>0) / cov:ignore-exempted /
#      uncovered. See scripts/lib/patch_coverage.py's module docstring for
#      the exact `cov:ignore:` scoping rule.
#
# In-scope crate decision: every file this script considers comes
# from `git diff`, which only ever names paths inside this repo — there is
# no separate crate allowlist to apply. All *.rs files under crates/ are in
# scope, including bench/test/example files: a benches/*.rs diff showing
# 0% coverage is not a bug in this script, it is the literal fact that a
# plain `cargo test --workspace` (and therefore `cargo llvm-cov --workspace`)
# never builds bench targets — see the earlier change's gate history,
# where exactly this happened to crates/raikiri-style/benches/cascade.rs and
# was escalated rather than silently passed.
#
# Usage: scripts/patch-coverage.sh [--lcov <path>] [BASE_REF]
#
#   --lcov <path>  Classify an existing lcov report instead of running the
#                  instrumented suite here (step 1 above is skipped). The
#                  caller is responsible for producing it from a clean
#                  `cargo llvm-cov --workspace --features raikiri-net/http-ureq`
#                  run of HEAD; scripts/gate.sh does this so the suite is
#                  built and run only once per gate run.
#   BASE_REF       Ref to compute the merge-base against (default: main).
#
# Env:
#   RAIKIRI_COVERAGE_INCLUDE_IGNORED=1   Also run `#[ignore]`d tests
#                                        (`-- --ignored`) as part of the
#                                        coverage run, accumulating profile
#                                        data across both invocations. Off
#                                        by default: doubles run time and
#                                        the VRT tests it adds require
#                                        scripts/wpt/fetch.sh.
#   RAIKIRI_GATE_TMPDIR                  See scripts/lib/tmpdir.sh.
#
# Exit status: 0 if every changed line is covered or cov:ignore-exempted,
# 1 if any changed line is uncovered (fix in-scope, or escalate to a
# follow-up item out-of-scope, then re-run), 2 if coverage
# measurement could not complete — a dirty tree, cargo-llvm-cov not
# installed, or a `cargo metadata` failure (see lib/patch_coverage.py's
# load_cargo_metadata) — which is an infra problem, not a coverage
# finding, and needs its own fix rather than a covering test.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Resolve REPO_ROOT from the caller's shell cwd, with a cross-tree
# mismatch guard — see scripts/lib/repo_root.sh for the rationale.
# shellcheck source=lib/repo_root.sh
source "$SCRIPT_DIR/lib/repo_root.sh"
cd "$REPO_ROOT"

# shellcheck source=lib/tmpdir.sh
source "$SCRIPT_DIR/lib/tmpdir.sh"

LCOV_IN=""
if [[ "${1:-}" == "--lcov" ]]; then
  if [[ -z "${2:-}" ]]; then
    echo "patch-coverage.sh: --lcov requires a path" >&2
    exit 2
  fi
  LCOV_IN="$2"
  shift 2
fi
BASE_REF="${1:-main}"
BASE_SHA="$(git merge-base "$BASE_REF" HEAD)"

# cargo-llvm-cov instruments the working tree, but the line-number
# classification below reads HEAD (`git diff <base> HEAD` for added lines,
# `git show HEAD:<path>` for cov:ignore scoping). On a dirty tree those two
# describe different file contents, so a coverage-vs-line mismatch would be
# silently reported as a confident verdict against misaligned line numbers.
# Refuse rather than guess.
if ! git diff --quiet HEAD -- '*.rs' || ! git diff --cached --quiet HEAD -- '*.rs'; then
  echo "patch-coverage.sh: working tree has uncommitted *.rs changes." >&2
  echo "  cargo-llvm-cov measures the working tree while the diff and" >&2
  echo "  cov:ignore scan read HEAD — on a dirty tree these describe" >&2
  echo "  different file contents. Commit or stash first." >&2
  exit 2
fi

if ! command -v cargo-llvm-cov >/dev/null 2>&1 && ! cargo llvm-cov --version >/dev/null 2>&1; then
  echo "patch-coverage.sh: cargo-llvm-cov not installed." >&2
  echo "  cargo install cargo-llvm-cov" >&2
  echo "  rustup component add llvm-tools" >&2
  exit 2
fi

LCOV_DIR="target/llvm-cov"
LCOV_OUT="$LCOV_DIR/patch-coverage.lcov"
mkdir -p "$LCOV_DIR"

echo "== scripts/patch-coverage.sh =="
echo "repo root : $REPO_ROOT"
echo "base ref  : $BASE_REF"
echo "base sha  : $BASE_SHA"
echo "TMPDIR    : $TMPDIR"
echo

# `cargo llvm-cov clean --workspace` first in both branches below: an
# incremental run reuses stale profile data and, after comment-only edits,
# reports false uncovered lines (off by one onto the line above a
# cov:ignore). A clean run re-builds and re-runs the whole workspace suite,
# so it is slower, but the line tables then match HEAD. The clean only
# touches this worktree's own target/ dir (TMPDIR scratch from
# scripts/lib/tmpdir.sh is unaffected), so concurrent worktrees just pay
# their own rebuild cost.
if [[ -n "$LCOV_IN" ]]; then
  if [[ ! -s "$LCOV_IN" ]]; then
    echo "patch-coverage.sh: --lcov file missing or empty: $LCOV_IN" >&2
    exit 2
  fi
  echo "-- using existing lcov report: $LCOV_IN --"
  LCOV_OUT="$LCOV_IN"
elif [[ "${RAIKIRI_COVERAGE_INCLUDE_IGNORED:-0}" == "1" ]]; then
  echo "-- cargo llvm-cov (normal + --ignored, accumulated) --"
  cargo llvm-cov clean --workspace
  # `--features raikiri-net/http-ureq`: raikiri-net's `http-ureq` feature
  # (UreqHttpProvider) is default off, so without this flag `ssrf_guard.rs` /
  # `http_resolver.rs` / `http_provider.rs` never compile here and every
  # executable line in them is reported as uncovered, matching the same flag
  # added to CI's coverage job (.github/workflows/ci.yml).
  cargo llvm-cov --workspace --features raikiri-net/http-ureq --locked --no-report
  cargo llvm-cov --workspace --features raikiri-net/http-ureq --locked --no-report -- --ignored
  cargo llvm-cov report --lcov --output-path "$LCOV_OUT"
else
  echo "-- cargo llvm-cov clean + --workspace --features raikiri-net/http-ureq --locked --lcov --"
  cargo llvm-cov clean --workspace
  cargo llvm-cov --workspace --features raikiri-net/http-ureq --locked --lcov --output-path "$LCOV_OUT"
fi
echo

echo "-- classifying changed lines --"
python3 "$SCRIPT_DIR/lib/patch_coverage.py" \
  --repo-root "$REPO_ROOT" \
  --base "$BASE_SHA" \
  --head HEAD \
  --lcov "$LCOV_OUT"
