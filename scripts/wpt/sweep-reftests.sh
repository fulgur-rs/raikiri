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
# The WPT cache is shared by every worktree. Record its revision before the
# sweep and refuse the result if another fetch moved it while tests ran.
wpt_sha="$(git -C target/wpt rev-parse HEAD)"
# The metadata names commits, so both trees must match them exactly.
require_clean() {
  if ! git -C "$1" diff --quiet HEAD --; then
    echo "$1 has uncommitted changes; the sweep would not match its commit" >&2
    exit 1
  fi
}
require_clean "$REPO_ROOT"
require_clean target/wpt

"$SCRIPT_DIR/list-runnable-reftests.sh" "$work_dir/runnable.txt" target/wpt

cargo run --release --locked -p raikiri-wpt --bin run-baseline-report -- \
  --wpt-root target/wpt \
  --baseline "$work_dir/runnable.txt" \
  --jobs "${WPT_SWEEP_JOBS:-$(nproc)}" \
  --output "$output"
# Record what was measured so the dashboard can refuse a sweep from another
# WPT pin and name the Raikiri revision it reflects.
require_clean target/wpt
if [[ "$(git -C target/wpt rev-parse HEAD)" != "$wpt_sha" ]]; then
  echo "WPT checkout moved from $wpt_sha during the sweep; rerun it" >&2
  rm -f "$output"
  exit 1
fi
{
  echo "wpt_sha=$wpt_sha"
  echo "raikiri_sha=$(git rev-parse HEAD)"
} > "$output.meta"
echo "Sweep written to $output (metadata in $output.meta)"
