#!/usr/bin/env bash
# scripts/cascade-bench-compare.sh — interleaved min-of-N cascade benchmark
# comparison (the related change).
#
# the related change landed crates/raikiri-style/benches/cascade.rs (a
# criterion regression guard for the cascade frequently executed loop's per-declaration
# constant) but deliberately did not wire it into anything that runs it:
# `cargo test --workspace` never builds bench targets and `cargo clippy
# --all-targets` only compiles, never executes, them. the related change
# is the follow-up that makes it runnable — as an explicit, opt-in script,
# not an automatic gate step (see the header of scripts/gate.sh's
# --with-bench flag and the "NOT connected to CI" note below).
#
# ── Why this is NOT a naive before/after comparison ────────────────────────
#
# benchmark change's own gate history (see the comments on the related change, and
# the "Measurement noise" section of benches/cascade.rs's module doc)
# measured that a single criterion `--baseline`/`--save-baseline` comparison
# is dominated by build contention from concurrent worktree sessions on
# this machine: **+18.8%/+11.5% on a tree with zero source changes**, one
# case reaching **+157%**. criterion's p-value cannot separate contention
# from a real regression — it only asks whether two sample sets came from
# the same distribution, and contention changes the distribution just as a
# real regression would.
#
# The validated fix (benchmark change's own experiment, reproduced the known +22.1%/
# +10.7% regression from the related change) is **interleaved min-of-N**:
# alternate N runs of the "good" (merge-base) and "bad" (HEAD) bench
# binaries and compare the *minimum* per-benchmark time across the N runs
# for each side, not the mean and not criterion's own verdict. Contention
# only ever *adds* latency, so the minimum converges toward the true cost
# while the mean does not. Interleaving (not just repetition) matters too:
# it keeps a slow stretch of machine from appearing entirely on one side.
#
# ── Threshold: 7%, chosen from the residual measurement variation and the weakest known signal ─
#
# Do NOT use the +18.8%/+11.5%/+157% figures above to justify a threshold —
# those are exactly what the min-of-N protocol exists to filter out; reusing
# them is the same reasoning error that produced the original filing's
# discredited "10%" (which fires on an unmodified tree, per the measurement
# above).
#
# The right inputs are the two numbers the *protocol itself* produces:
#
#   - Floor (residual noise after the fix): min-of-N results on an
#     unmodified tree, measured a second time at a nearby base commit,
#     agreed to 2.0% (rule_heavy) and 3.3% (element_heavy) — see
#     benches/cascade.rs's module doc, "Healthy baseline" paragraph.
#   - Ceiling (weakest legitimate signal to catch): the smaller of the two
#     known real regressions under this same protocol, +10.7%
#     (element_heavy, n=8 interleaved) — see the same module doc's
#     "validated" table.
#
# 7% sits above the ~3.3% floor (≈2x headroom against protocol noise) and
# below the 10.7% ceiling (≈35% margin before the weakest known signal),
# applied **per benchmark** (any one benchmark exceeding it fails the
# comparison).
#
# ── Invocation is pinned, not just the threshold ───────────────────────────
#
# the related change's gate history also found that criterion's
# `iter_batched`-family `batch_size = ceil(iters / 10)` changes the
# memory-reuse regime depending on `--sample-size`/`--measurement-time`
# (module doc: batch ≤5 under this script's flags vs batch ≤10 under plain
# `cargo bench` defaults). The +22.1%/+10.7% conclusion is unaffected
# (symmetric on both sides of the comparison), but a *threshold* is
# meaningless without the invocation pinned alongside it — so the criterion
# flags below (CRITERION_FLAGS) are hardcoded, not a default a caller can
# silently drift. N and the threshold are deliberately still overridable
# (see Env below): unlike the criterion flags, they don't change what is
# being measured, only how many samples and how much delta is tolerated.
#
# Usage: scripts/cascade-bench-compare.sh [BASE_REF]
#
#   BASE_REF   Ref to compute the merge-base ("good" side) against
#              (default: main).
#
# Env:
#   RAIKIRI_CASCADE_N              Override N (default: 8).
#   RAIKIRI_CASCADE_THRESHOLD_PCT  Override the threshold (default: 7).
#   RAIKIRI_GATE_TMPDIR            See scripts/lib/tmpdir.sh.
#
# NOT connected to CI, a cargo profile, or scripts/gate.sh's default run —
# the related change's own filing flags that path as a package-boundary rule
# (shared build prerequisites) crossing requiring escalation this script is
# not authorized to make. scripts/gate.sh only invokes this when given the
# explicit --with-bench flag.
#
# Exit status: 0 = every benchmark's min-of-N delta is within threshold (or
# the bench target doesn't exist at the merge-base, in which case the
# comparison is skipped, not failed). 1 = at least one benchmark exceeded
# the threshold, or a automated-check error occurred (a build failure surfaces
# cargo's own exit status via `set -e`; a missing benchmark report from
# scripts/lib/cascade_compare.py surfaces its own exit code, 1 or 2 — see
# that file for specifics). Either way, non-zero means "do not treat this
# as a clean comparison".

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Resolve REPO_ROOT from the caller's shell cwd, with a cross-tree
# mismatch guard — see scripts/lib/repo_root.sh for the rationale.
# shellcheck source=lib/repo_root.sh
source "$SCRIPT_DIR/lib/repo_root.sh"
cd "$REPO_ROOT"

# shellcheck source=lib/tmpdir.sh
source "$SCRIPT_DIR/lib/tmpdir.sh"

BASE_REF="${1:-main}"
BASE_SHA="$(git merge-base "$BASE_REF" HEAD)"
N="${RAIKIRI_CASCADE_N:-8}"
THRESHOLD_PCT="${RAIKIRI_CASCADE_THRESHOLD_PCT:-7}"

# Bounds validation (review finding): these overrides previously
# had none. An unbounded N has no cap on runtime or disk usage proportional
# to whatever a caller sets. An unbounded/unvalidated threshold is worse:
# Python's `float()` happily parses "nan" and "inf", and since
# `delta_pct > threshold` in cascade_compare.py is always False when
# threshold is NaN or +inf, an unvalidated threshold could silently PASS an
# arbitrarily large regression. Fail fast, before any build work, on an
# invalid value. cascade_compare.py validates its own `--n`/`--threshold-pct`
# independently (it can be invoked directly, bypassing this script).
#
# The digit count is capped directly in the regex (`[0-9]?`, i.e. at most 2
# digits) rather than matching an unbounded run of digits and rejecting via
# `-gt 50` afterward: bash's `[[ -gt ]]` is fixed-width arithmetic, and an
# absurdly long digit string (still a valid match for an unbounded
# `[0-9]*`) can overflow that arithmetic and wrap around, potentially
# passing the `-gt 50` check it was meant to fail. Bounding the digit count
# in the regex itself means the arithmetic comparison only ever sees a
# value in [1, 99], where overflow cannot occur. review finding.
if ! [[ "$N" =~ ^[1-9][0-9]?$ ]] || [[ "$N" -gt 50 ]]; then
  echo "cascade-bench-compare.sh: RAIKIRI_CASCADE_N must be a positive integer <= 50 (got: '$N')" >&2
  exit 2
fi
if ! python3 -c "
import math, sys
try:
    v = float(sys.argv[1])
except ValueError:
    sys.exit(1)
sys.exit(0 if math.isfinite(v) and 0 <= v <= 100 else 1)
" "$THRESHOLD_PCT"; then
  echo "cascade-bench-compare.sh: RAIKIRI_CASCADE_THRESHOLD_PCT must be a finite number in [0, 100] (got: '$THRESHOLD_PCT')" >&2
  exit 2
fi

# Pinned criterion invocation (see header: threshold is meaningless without
# this pinned alongside it).
CRITERION_FLAGS=(--warm-up-time 0.5 --measurement-time 1.2 --sample-size 10 --noplot)

BENCH_FILE="crates/raikiri-style/benches/cascade.rs"

echo "== scripts/cascade-bench-compare.sh =="
echo "repo root : $REPO_ROOT"
echo "base ref  : $BASE_REF"
echo "base sha  : $BASE_SHA"
echo "N         : $N"
echo "threshold : ${THRESHOLD_PCT}%"
echo "TMPDIR    : $TMPDIR"
if command -v nproc >/dev/null 2>&1; then
  echo "nproc     : $(nproc)"
fi
if [[ -r /proc/cpuinfo ]]; then
  echo "cpu model : $(grep -m1 'model name' /proc/cpuinfo | cut -d: -f2- | sed 's/^ *//')"
fi
if command -v pgrep >/dev/null 2>&1; then
  echo "concurrent rustc processes at start: $(pgrep -c rustc || true)"
fi
echo

if ! git cat-file -e "$BASE_SHA:$BENCH_FILE" 2>/dev/null; then
  echo "SKIP: $BENCH_FILE does not exist at $BASE_SHA — nothing to compare"
  echo "      against (this base predates the benchmark implementation merge"
  echo "      the bench). Not a failure."
  exit 0
fi

SCRATCH="$(mktemp -d "$TMPDIR/raikiri-cascade-bench.XXXXXX")"
GOOD_WORKTREE="$SCRATCH/good-tree"
GOOD_CRIT="$SCRATCH/crit-good"
BAD_CRIT="$SCRATCH/crit-bad"
mkdir -p "$GOOD_CRIT" "$BAD_CRIT"

cleanup() {
  git -C "$REPO_ROOT" worktree remove --force "$GOOD_WORKTREE" >/dev/null 2>&1 || true
  rm -rf "$SCRATCH"
}
trap cleanup EXIT

# ── locate a compiler-artifact's executable path from `--message-format=json` ─
find_bench_executable() {
  # $1: path to the cargo --message-format=json output
  python3 - "$1" <<'PYEOF'
import json, sys
path = sys.argv[1]
with open(path, encoding="utf-8") as f:
    for line in f:
        line = line.strip()
        if not line:
            continue
        obj = json.loads(line)
        if obj.get("reason") == "compiler-artifact" and obj.get("target", {}).get("name") == "cascade":
            exe = obj.get("executable")
            if exe:
                print(exe)
                sys.exit(0)
sys.exit(1)
PYEOF
}

echo "-- building bad (HEAD) bench binary in $REPO_ROOT --"
BAD_JSON="$SCRATCH/bad-build.json"
cargo bench -p raikiri-style --bench cascade --no-run --locked --message-format=json \
  > "$BAD_JSON"
BAD_EXE="$(find_bench_executable "$BAD_JSON")"
echo "bad exe: $BAD_EXE"
echo

echo "-- checking out merge-base $BASE_SHA into a detached worktree --"
git worktree add --detach "$GOOD_WORKTREE" "$BASE_SHA" >/dev/null
echo "-- building good ($BASE_SHA) bench binary in $GOOD_WORKTREE (separate target dir) --"
GOOD_JSON="$SCRATCH/good-build.json"
(cd "$GOOD_WORKTREE" && cargo bench -p raikiri-style --bench cascade --no-run --locked --message-format=json) \
  > "$GOOD_JSON"
GOOD_EXE="$(find_bench_executable "$GOOD_JSON")"
echo "good exe: $GOOD_EXE"
echo

# Copy binaries out so they survive regardless of what happens to either
# tree's target/ afterward (the worktree is removed on exit).
cp "$BAD_EXE" "$SCRATCH/cascade-bad"
cp "$GOOD_EXE" "$SCRATCH/cascade-good"
chmod +x "$SCRATCH/cascade-bad" "$SCRATCH/cascade-good"

echo "-- interleaving $N runs of each (pinned flags: ${CRITERION_FLAGS[*]}) --"
for ((i = 0; i < N; i++)); do
  echo "  run $i: good"
  CRITERION_HOME="$GOOD_CRIT/$i" "$SCRATCH/cascade-good" --bench "${CRITERION_FLAGS[@]}" >/dev/null
  echo "  run $i: bad"
  CRITERION_HOME="$BAD_CRIT/$i" "$SCRATCH/cascade-bad" --bench "${CRITERION_FLAGS[@]}" >/dev/null
done
echo

echo "-- per-benchmark min-of-N --"
python3 "$SCRIPT_DIR/lib/cascade_compare.py" \
  --good-dir "$GOOD_CRIT" \
  --bad-dir "$BAD_CRIT" \
  --n "$N" \
  --threshold-pct "$THRESHOLD_PCT"
