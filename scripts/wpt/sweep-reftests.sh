#!/usr/bin/env bash
# Run every structurally runnable WPT reftest once and write a
# `run-baseline-report` TSV for the dashboard. This is report-only: no
# expectations file changes, and FAIL / ERROR rows do not fail the command.
#
# Usage: scripts/wpt/sweep-reftests.sh [OUTPUT.tsv]
#   (default output: target/wpt-sweep/sweep-<raikiri short SHA>.tsv, so the
#   dashboard names the revision it measured; WPT_SWEEP_JOBS sets --jobs)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../lib/repo_root.sh
source "$SCRIPT_DIR/../lib/repo_root.sh"

cd "$REPO_ROOT"
output="${1:-$REPO_ROOT/target/wpt-sweep/sweep-$(git rev-parse --short HEAD).tsv}"
work_dir="$REPO_ROOT/target/wpt-sweep"
mkdir -p "$work_dir" "$(dirname "$output")"

"$SCRIPT_DIR/fetch.sh"

python3 "$SCRIPT_DIR/survey_reftests.py" --wpt-root target/wpt --format json \
  --output "$work_dir/survey.json"
python3 - "$work_dir/survey.json" "$work_dir/runnable.txt" <<'PY'
import json
import sys

report = json.load(open(sys.argv[1], encoding="utf-8"))
ids = sorted(
    test["test_id"]
    for category in report["categories"].values()
    for theme in category["themes"]
    for test in theme["tests"]
    if test["structurally_runnable"]
)
with open(sys.argv[2], "w", encoding="utf-8") as out:
    out.write("".join(f"{test_id}\n" for test_id in ids))
print(f"{len(ids)} runnable reftests")
PY

cargo run --release --locked -p raikiri-wpt --bin run-baseline-report -- \
  --wpt-root target/wpt \
  --baseline "$work_dir/runnable.txt" \
  --jobs "${WPT_SWEEP_JOBS:-$(nproc)}" \
  --output "$output"
echo "Sweep written to $output"
