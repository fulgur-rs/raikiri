#!/usr/bin/env bash
# scripts/gate.sh — mechanical §8.1 automated check, in one command.
#
# Implements `../raikiri-workflow/rules/gate.md` §8.1 (a)(b), plus the
# §8.1.4 applicability predicate. §8.1(c) (patch coverage) and the optional
# cascade benchmark step are separate scripts this file shells out to once
# they exist (bd raikiri-spike-wvch / raikiri-spike-iebo); see --help.
#
# This script is *tooling*, not the rule itself: it encodes what gate.md
# already says (cite the section on any behavior change) so an agent gets
# it right by running one command instead of re-deriving each check from
# prose every time. If gate.md's text changes, this script should follow —
# not the other way around.
#
# Usage: scripts/gate.sh [--base <ref>] [--skip-coverage] [--with-bench]
#
#   --base <ref>       Base ref for the §8.1.4 applicability diff and for
#                       patch coverage's merge-base (default: main).
#   --skip-coverage     Skip §8.1(c) patch coverage (bd raikiri-spike-wvch).
#                       Useful for a fast fmt/clippy/test-only iteration loop;
#                       do not skip before an actual gate pass declaration.
#   --with-bench        Also run the cascade benchmark comparison
#                       (bd raikiri-spike-iebo) against --base. This is
#                       deliberately NOT part of the default run — it is an
#                       explicit, human/coordinator-invoked step, not
#                       something CI or this script's default path runs
#                       automatically (walls.md 壁 7 escalation boundary).
#
# Exit status: 0 if every applicable check passed (or §8.1.4 judged the
# whole thing non-applicable), non-zero otherwise. Output is intentionally
# verbose — it is itself the gate record (`N passed`, census output, etc.)
# gate.md's 記録義務 requires quoting verbatim.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && git rev-parse --show-toplevel)"
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
      sed -n '2,32p' "${BASH_SOURCE[0]}"
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

# ── §8.1.4 non-applicability test (mechanical) ─────────────────────────────
#
# gate.md §8.1.4: skip §8.1/§8.1.2/§8.1.3 iff (1) the name-only diff against
# merge-base has zero matches for the pattern below AND (2) the diff has no
# entries the pattern can't judge by path name alone (renames' source side,
# gitlinks, symlink retargets — --no-renames handles the common rename case
# mechanically; the remaining "path doesn't encode content" cases are not
# mechanically enumerable and are left to the caller per §8.1.4's own
# "判定手法の適用限界" clause — this script only automates condition (1)).
echo "-- §8.1.4 applicability test --"
APPLICABILITY_DIFF="$(git diff --no-renames --name-only "$BASE_REF"...HEAD)"
echo "git diff --no-renames --name-only $BASE_REF...HEAD"
if [[ -z "$APPLICABILITY_DIFF" ]]; then
  echo "(empty diff)"
else
  echo "$APPLICABILITY_DIFF"
fi

if echo "$APPLICABILITY_DIFF" | grep -qE '\.rs$|(^|/)Cargo\.(toml|lock)$|(^|/)expectations/.*\.txt$'; then
  APPLICABLE=1
  echo "=> match found, §8.1 applies"
else
  APPLICABLE=0
  echo "=> no match on file names alone. Condition (2) of §8.1.4 (path-name"
  echo "   applicability limits: renames' source side already handled by"
  echo "   --no-renames above; gitlinks / symlink retargets are NOT checked"
  echo "   by this script and must be confirmed by the caller before"
  echo "   treating this as a real skip) is NOT verified mechanically here."
fi
echo

# ── optional: cascade benchmark comparison (bd raikiri-spike-iebo) ─────────
#
# Deliberately gated behind --with-bench, and deliberately run *before* the
# §8.1.4 applicability early-exit below: --with-bench is not part of §8.1
# and the §8.1.4 predicate (a Rust-diff test) has no authority over it. A
# caller who explicitly asks for the bench comparison must get it run (or
# see an explicit note that it wasn't) regardless of what §8.1.4 decides —
# an early exit that silently drops an explicitly-requested step is exactly
# the vacuous-green shape gate.md's own §8.1(a) discussion warns about.
#
# This is NOT part of §8.1 proper (rules/gate.md has not been changed to
# require it — that would need a retro → bd decision → human approve) and
# is NOT wired into CI or any cargo profile. It exists so a coordinator/gate
# reviewer can invoke it on demand when a diff touches
# crates/raikiri-style/src/cascade.rs or adjacent hot-loop code.
if [[ "$WITH_BENCH" -eq 1 ]]; then
  if [[ -x "$SCRIPT_DIR/cascade-bench-compare.sh" ]]; then
    echo "-- optional: cascade benchmark comparison --"
    if ! "$SCRIPT_DIR/cascade-bench-compare.sh" "$BASE_REF"; then
      echo "FAIL: cascade-bench-compare.sh reported a regression past threshold"
      FAIL=1
    fi
  else
    echo "-- optional: --with-bench passed but scripts/cascade-bench-compare.sh"
    echo "   not found (bd raikiri-spike-iebo not yet landed in this tree) --"
    FAIL=1
  fi
  echo
fi

if [[ "$APPLICABLE" -eq 0 ]]; then
  echo "§8.1/§8.1.2/§8.1.3 judged non-applicable by condition (1)."
  echo "Record this output plus your own condition-(2) confirmation as the"
  echo "§8.1.4 skip rationale (gate.md 記録義務)."
  if [[ "$WITH_BENCH" -eq 1 ]]; then
    echo "note: --with-bench was requested and ran above — §8.1.4 has no"
    echo "      authority over it, only over §8.1/§8.1.2/§8.1.3 below."
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

# ── §8.1(a) test execution ─────────────────────────────────────────────────
echo "-- §8.1(a) cargo test --workspace --locked --"
if ! cargo test --workspace --locked; then
  echo "FAIL: cargo test --workspace --locked"
  FAIL=1
fi
echo

echo "-- §8.1(a) #[ignore] census --"
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
echo "census matches: $CENSUS_COUNT"
echo

if [[ "$CENSUS_COUNT" -eq 0 ]]; then
  echo "-- §8.1(a) --ignored run: skipped (census is 0; this line is the"
  echo "   0-count assertion's satisfaction record per gate.md §8.1(a).2) --"
  IGNORED_N=0
else
  # gate.md §8.1(a): "-- --ignored" needs target/wpt (VRT font fixtures)
  # fetched. In a linked worktree, target/ is per-worktree and doesn't
  # inherit main's fetch (bd raikiri-spike-dz8t). Auto-repair the symlink
  # rather than fail the whole gate on a one-line environment gap — this is
  # exactly the "record verified by re-running it" spirit bd raikiri-spike-
  # weky's own comment asks for; a gate that requires a silent manual
  # `ln -s` before it works is the same failure class as the tmpfs bug.
  if [[ ! -e "$REPO_ROOT/target/wpt" ]]; then
    MAIN_WORKTREE="$(git worktree list --porcelain | awk '/^worktree /{print $2; exit}')"
    if [[ -n "$MAIN_WORKTREE" && "$MAIN_WORKTREE" != "$REPO_ROOT" && -e "$MAIN_WORKTREE/target/wpt" ]]; then
      mkdir -p "$REPO_ROOT/target"
      ln -s "$MAIN_WORKTREE/target/wpt" "$REPO_ROOT/target/wpt"
      echo "(re-linked target/wpt -> $MAIN_WORKTREE/target/wpt, bd raikiri-spike-dz8t)"
    else
      echo "warning: target/wpt missing and no main worktree fetch found to"
      echo "         link against. Run scripts/wpt/fetch.sh first if the"
      echo "         --ignored run below fails on VRT font fixtures."
    fi
  fi

  echo "-- §8.1(a) cargo test --workspace --locked -- --ignored --"
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
  # Sum "N passed" across every harness's "test result:" summary line —
  # gate.md 記録義務: this is the sum the record must cite, not any single
  # harness's line and not the normal run's "N ignored" figure.
  IGNORED_N=$(echo "$IGNORED_OUTPUT" | grep -oE '[0-9]+ passed' | grep -oE '[0-9]+' | awk '{s+=$1} END {print s+0}')
  echo "N passed (summed across all harness summary lines): $IGNORED_N"
  if [[ "$IGNORED_N" -eq 0 ]]; then
    echo "FAIL: N passed == 0 on a non-zero census (gate.md §8.1(a): N=0 is"
    echo "      'not run', not a pass)"
    FAIL=1
  fi
fi
echo

# ── §8.1(b) static checks ───────────────────────────────────────────────────
echo "-- §8.1(b) cargo fmt --all --check --"
if ! cargo fmt --all --check; then
  echo "FAIL: cargo fmt --all --check"
  FAIL=1
fi
echo

echo "-- §8.1(b) cargo clippy --workspace --all-targets -- -D warnings --"
if ! cargo clippy --workspace --all-targets --locked -- -D warnings; then
  echo "FAIL: cargo clippy --workspace --all-targets --locked -- -D warnings"
  FAIL=1
fi
echo

echo "-- §8.1(b) RUSTDOCFLAGS=\"-D warnings\" cargo doc --no-deps --workspace --"
if ! RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --workspace --locked; then
  echo "FAIL: cargo doc --no-deps --workspace"
  FAIL=1
fi
echo

# NB: §8.1(b)'s "expectations/*.txt lint (WPT status file consistency)" has
# no dedicated script in this repo yet (raikiri-wpt's validate-expectations
# binary covers a related but not identical surface — see
# crates/raikiri-wpt/src/bin/validate-expectations.rs). Out of scope for bd
# raikiri-spike-weky/wvch/iebo; flagged here rather than silently omitted.
echo "-- §8.1(b) expectations/*.txt lint --"
echo "NOT AUTOMATED by this script yet (no dedicated lint entry point found)."
echo "See crates/raikiri-wpt/src/bin/validate-expectations.rs for the closest"
echo "existing tool and confirm manually until this is wired in."
echo

# ── §8.1(c) patch coverage ───────────────────────────────────────────────────
if [[ "$SKIP_COVERAGE" -eq 1 ]]; then
  echo "-- §8.1(c) patch coverage: --skip-coverage passed, skipping --"
elif [[ -x "$SCRIPT_DIR/patch-coverage.sh" ]]; then
  echo "-- §8.1(c) patch coverage (scripts/patch-coverage.sh --base $BASE_REF) --"
  if ! "$SCRIPT_DIR/patch-coverage.sh" "$BASE_REF"; then
    echo "FAIL: scripts/patch-coverage.sh reported uncovered changed lines"
    echo "      without a cov:ignore escape. Per gate.md §8.1.1: add a"
    echo "      covering test (in-scope) or escalate to a bd issue"
    echo "      (out-of-scope), then re-run."
    FAIL=1
  fi
else
  echo "-- §8.1(c) patch coverage: scripts/patch-coverage.sh not found --"
  echo "   (bd raikiri-spike-wvch not yet landed in this tree). Patch"
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
