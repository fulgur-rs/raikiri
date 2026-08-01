#!/usr/bin/env bash
# scripts/patch-coverage.sh — gate.md §8.1.1 patch coverage, measured.
#
# bd raikiri-spike-wvch: §8.1.1 has required "100% of changed lines covered
# or escalated" since it was written, but nothing in this repo could measure
# it — cargo-llvm-cov was not installed and this script did not exist, so
# every task degraded to manual enumeration + lens spot-check (documented
# across 4 consecutive sprint retros, style-12 through style-14).
#
# Approach (workspace lcov → merge-base diff → gated-crate judgment, per
# gate.md §8.1.1, modeled on flpdf's patch-coverage.sh):
#
#   1. Run the *whole workspace* test suite under cargo-llvm-cov
#      instrumentation, export lcov.
#   2. Diff merge-base(<base>, HEAD)..HEAD for *.rs, in -U0 form, to get the
#      exact added-line set per file (git.md §8.1.4's canonical merge-base
#      handling: base is resolved once via `git merge-base`, not a bare
#      ref, so a concurrent landing on <base> after this branch forked
#      cannot change what counts as "this diff's lines").
#   3. For each added line: covered (lcov hit>0) / cov:ignore-exempted /
#      uncovered. See scripts/lib/patch_coverage.py's module docstring for
#      the exact `cov:ignore:` scoping rule.
#
# "gated crate 判定" (gate.md §51): every file this script considers comes
# from `git diff`, which only ever names paths inside this repo — there is
# no separate crate allowlist to apply. All *.rs files under crates/ are in
# scope, including bench/test/example files: a benches/*.rs diff showing
# 0% coverage is not a bug in this script, it is the literal fact that a
# plain `cargo test --workspace` (and therefore `cargo llvm-cov --workspace`)
# never builds bench targets — see bd raikiri-spike-iebo's gate history,
# where exactly this happened to crates/raikiri-style/benches/cascade.rs and
# was escalated rather than silently passed.
#
# Usage: scripts/patch-coverage.sh [BASE_REF]
#
#   BASE_REF   Ref to compute the merge-base against (default: main).
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
# 1 if any changed line is uncovered (gate.md §8.1.1: fix in-scope, or
# escalate to a bd issue out-of-scope, then re-run).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && git rev-parse --show-toplevel)"
cd "$REPO_ROOT"

# shellcheck source=lib/tmpdir.sh
source "$SCRIPT_DIR/lib/tmpdir.sh"

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

if [[ "${RAIKIRI_COVERAGE_INCLUDE_IGNORED:-0}" == "1" ]]; then
  echo "-- cargo llvm-cov (normal + --ignored, accumulated) --"
  cargo llvm-cov clean --workspace
  cargo llvm-cov --workspace --locked --no-report
  cargo llvm-cov --workspace --locked --no-report -- --ignored
  cargo llvm-cov report --lcov --output-path "$LCOV_OUT"
else
  echo "-- cargo llvm-cov --workspace --locked --lcov --"
  cargo llvm-cov --workspace --locked --lcov --output-path "$LCOV_OUT"
fi
echo

echo "-- classifying changed lines --"
python3 "$SCRIPT_DIR/lib/patch_coverage.py" \
  --repo-root "$REPO_ROOT" \
  --base "$BASE_SHA" \
  --head HEAD \
  --lcov "$LCOV_OUT"
