#!/usr/bin/env bash
# Verify safe_merge.sh accepts only complete, current evidence records.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SAFE_MERGE="$SCRIPT_DIR/safe_merge.sh"
TMP_ROOT="${TMPDIR:-/tmp}"
if [[ ! -d "$TMP_ROOT" || ! -w "$TMP_ROOT" ]]; then
  TMP_ROOT="/tmp"
fi
TMP_DIR="$(mktemp -d "${TMP_ROOT%/}/safe-merge-test.XXXXXX")"
trap 'rm -rf "$TMP_DIR"' EXIT

assert_accepts() {
  local name="$1" message="$2"
  local output="$TMP_DIR/$name.out"
  if ! "$SAFE_MERGE" main -m "$message" --check-only >"$output" 2>&1; then
    echo "FAIL: safe_merge.sh rejected $name evidence" >&2
    cat "$output" >&2
    exit 1
  fi
}

assert_rejects() {
  local name="$1" message="$2"
  local output="$TMP_DIR/$name.out"
  if "$SAFE_MERGE" main -m "$message" --check-only >"$output" 2>&1; then
    echo "FAIL: safe_merge.sh accepted $name evidence" >&2
    cat "$output" >&2
    exit 1
  fi
}

assert_accepts current-pass $'Merge checks\n- Validation: PASS (CI green)\n- Independent review: PASS (Roborev #404)\n- Patch coverage: PASS'
assert_accepts current-na $'Merge checks\n- Validation: PASS\n- Independent review: PASS\n- Patch coverage: N/A (no changed executable lines)'
assert_rejects duplicate-validation $'Merge checks\n- Validation: PASS\n- Validation: PASS\n- Independent review: PASS\n- Patch coverage: PASS'
assert_rejects retired-checklist $'Merge checks\n- §8.1: PASS\n- §8.2: (i)\n- §8.3: task-check-123 GATE PASS\n- §8.1.1: covered'
assert_rejects failed-validation $'Merge checks\n- Validation: FAIL\n- Independent review: PASS\n- Patch coverage: PASS'

echo "safe_merge.sh evidence tests: PASS"
