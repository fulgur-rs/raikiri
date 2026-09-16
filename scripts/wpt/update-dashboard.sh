#!/usr/bin/env bash
# Fetch the pinned WPT tree, verify the smoke harness, and regenerate the
# tracked CSS coverage dashboard from the current T2 baseline.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../lib/repo_root.sh
source "$SCRIPT_DIR/../lib/repo_root.sh"

cd "$REPO_ROOT"
"$SCRIPT_DIR/fetch.sh"

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
  --output "$REPO_ROOT/docs/wpt-dashboard.html"
