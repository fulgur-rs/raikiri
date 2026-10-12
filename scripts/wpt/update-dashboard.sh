#!/usr/bin/env bash
# Fetch the pinned WPT tree, verify the smoke test runner, and regenerate the
# tracked CSS coverage dashboard.
#
# Usage: scripts/wpt/update-dashboard.sh [--sweep FILE | --run-sweep]
#   --sweep FILE  use an existing run-baseline-report TSV for PASS/FAIL/ERROR
#   --run-sweep   run scripts/wpt/sweep-reftests.sh first (slow) and use it
# Without either, the page shows reftest counts and baseline coverage only.
set -euo pipefail

sweep=""
run_sweep=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --sweep) sweep="${2:?--sweep needs a file}"; shift 2 ;;
    --run-sweep) run_sweep=1; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../lib/repo_root.sh
source "$SCRIPT_DIR/../lib/repo_root.sh"

cd "$REPO_ROOT"
"$SCRIPT_DIR/fetch.sh"
if [[ "$run_sweep" -eq 1 ]]; then
  sweep="$REPO_ROOT/target/wpt-sweep/sweep-$(git rev-parse --short HEAD).tsv"
  "$SCRIPT_DIR/sweep-reftests.sh" "$sweep"
fi
sweep_args=()
if [[ -n "$sweep" ]]; then
  sweep_args=(--sweep "$sweep")
fi

# Keep the smoke result in an ignored target file so the generated page can
# report the count without making ordinary tests or tracked files stateful.
test_log="$REPO_ROOT/target/basic-reftests-dashboard.log"
trap 'rm -f "$test_log"' EXIT
cargo test --locked -p raikiri-wpt --test basic_reftests 2>&1 | tee "$test_log"
basic_reftest_count="$(sed -nE 's/^test result: ok\. ([0-9]+) passed;.*/\1/p' "$test_log" | tail -n 1)"
if [[ -z "$basic_reftest_count" ]]; then
  echo "could not determine the basic reftest pass count" >&2
  exit 1
fi

cargo run --locked -p raikiri-wpt --bin generate-dashboard -- \
  --wpt-root "$REPO_ROOT/target/wpt" \
  --baseline "$REPO_ROOT/expectations/raikiri-baseline.txt" \
  --basic-reftest-count "$basic_reftest_count" \
  "${sweep_args[@]}" \
  --output "$REPO_ROOT/docs/wpt-dashboard.html"
