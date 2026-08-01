#!/usr/bin/env python3
"""scripts/lib/cascade_compare.py — interleaved min-of-N bench comparison.

Called by `scripts/cascade-bench-compare.sh` after it has produced N
per-variant criterion result directories, laid out as:

    <good_dir>/<run index 0..N-1>/cascade/<bench name>/new/estimates.json
    <bad_dir>/<run index 0..N-1>/cascade/<bench name>/new/estimates.json

one directory per invocation of the bench binary (a fresh `CRITERION_HOME`
per run so nothing overwrites a prior run's `estimates.json` before it is
read).

# Why "slope", not "mean" or the CLI's raw numbers

criterion's console line (`time: [X Y Z]`) is not the `mean` field in
`estimates.json` — it is the `slope` field. Verified directly: a sample run
of `cascade/rule_heavy_50x500` printed `time: [5.1188 ms 5.1504 ms
5.1778 ms]`, and only `estimates.json`'s `slope.{lower_bound,point_estimate,
upper_bound}` matched those three numbers to 4 significant figures (5.1188 /
5.1504 / 5.1778 ms exactly); `mean` did not (5.136 / 5.185 / 5.248 ms). This
script reads `slope.point_estimate` (nanoseconds) per run, to match what a
human reading the same console output would call "the time".

# Why "minimum across runs", not any single run's estimate

bd raikiri-spike-iebo's gate history measured that a single `--baseline`/
`--save-baseline` comparison is dominated by build/rustc contention from
concurrent worktree sessions on this machine (+18.8%/+11.5% on a tree with
*zero* source changes, one case reaching +157%). Contention only ever adds
latency, so the minimum over N interleaved runs converges toward the true
cost while the mean does not. See the module doc of
`crates/raikiri-style/benches/cascade.rs` for the full validation (n=8
interleaved reproduced the origin's known +22.1%/+10.7% regression while a
same-tree control stayed near zero).
"""

from __future__ import annotations

import argparse
import json
import math
import sys
from pathlib import Path

# Bounds enforced independently of scripts/cascade-bench-compare.sh's own
# validation, since this module can be invoked directly (Codex §8.3 review
# finding: env-var overrides had no validation at all — argparse's plain
# `type=int`/`type=float` accept e.g. "nan"/"inf" for the threshold, under
# which `delta_pct > threshold` is always False and a real regression would
# silently PASS; an unbounded N has no cap on run count).
MAX_N = 50
MAX_THRESHOLD_PCT = 100.0


def positive_int_capped(s: str) -> int:
    v = int(s)  # ValueError (non-integer input) is handled by argparse itself
    if v < 1 or v > MAX_N:
        raise argparse.ArgumentTypeError(f"must be a positive integer <= {MAX_N}, got {v}")
    return v


def finite_percentage(s: str) -> float:
    v = float(s)  # ValueError is handled by argparse itself
    if not math.isfinite(v) or v < 0 or v > MAX_THRESHOLD_PCT:
        raise argparse.ArgumentTypeError(
            f"must be a finite number in [0, {MAX_THRESHOLD_PCT}], got {v}"
        )
    return v


def load_slope_ns(estimates_path: Path) -> float:
    with open(estimates_path, encoding="utf-8") as f:
        data = json.load(f)
    return float(data["slope"]["point_estimate"])


def collect_runs(variant_dir: Path, bench_name: str, n: int) -> list[float]:
    values = []
    for i in range(n):
        p = variant_dir / str(i) / "cascade" / bench_name / "new" / "estimates.json"
        if not p.is_file():
            raise FileNotFoundError(f"missing {p} (run {i} of {bench_name} did not produce a report)")
        values.append(load_slope_ns(p))
    return values


def discover_bench_names(variant_dir: Path) -> list[str]:
    run0 = variant_dir / "0" / "cascade"
    if not run0.is_dir():
        raise FileNotFoundError(f"{run0} not found — did run 0 execute?")
    return sorted(p.name for p in run0.iterdir() if (p / "new" / "estimates.json").is_file())


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--good-dir", required=True, type=Path)
    ap.add_argument("--bad-dir", required=True, type=Path)
    ap.add_argument("--n", required=True, type=positive_int_capped)
    ap.add_argument("--threshold-pct", required=True, type=finite_percentage)
    args = ap.parse_args()

    good_names = discover_bench_names(args.good_dir)
    bad_names = discover_bench_names(args.bad_dir)
    if good_names != bad_names:
        print(f"error: benchmark name sets differ: good={good_names} bad={bad_names}", file=sys.stderr)
        return 2
    if not good_names:
        print("error: no benchmarks discovered", file=sys.stderr)
        return 2

    print(f"{'benchmark':<24}{'good min (ms)':>16}{'bad min (ms)':>16}{'delta':>10}{'spread (good)':>16}")
    fail = False
    for name in good_names:
        good_vals = collect_runs(args.good_dir, name, args.n)
        bad_vals = collect_runs(args.bad_dir, name, args.n)
        good_min = min(good_vals)
        bad_min = min(bad_vals)
        delta_pct = (bad_min - good_min) / good_min * 100.0
        # Informational spread indicator: (max-min)/min across the N good-side
        # runs' own point estimates — NOT compared against the delta or the
        # threshold (they measure different things: this is how noisy the N
        # good-side *runs* were, delta is min-vs-min between good and bad).
        # Context only, since the module doc's own noise floor (2.0%/3.3%)
        # was measured the same way.
        spread_pct = (max(good_vals) - min(good_vals)) / min(good_vals) * 100.0
        status = "FAIL" if delta_pct > args.threshold_pct else "ok"
        if delta_pct > args.threshold_pct:
            fail = True
        print(
            f"{name:<24}{good_min / 1e6:>16.4f}{bad_min / 1e6:>16.4f}"
            f"{delta_pct:>9.2f}%{spread_pct:>15.2f}%  {status}"
        )

    print()
    print("spread (good) is (max-min)/min across the N good-side runs — noise")
    print("context, not a pass/fail signal; only the delta column is compared")
    print("against the threshold.")
    print(f"threshold: {args.threshold_pct:.1f}% (see scripts/cascade-bench-compare.sh header for justification)")
    if fail:
        print("FAIL: at least one benchmark's min-of-N delta exceeds the threshold")
        return 1
    print("PASS: no benchmark's min-of-N delta exceeds the threshold")
    return 0


if __name__ == "__main__":
    sys.exit(main())
