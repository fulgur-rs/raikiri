#!/usr/bin/env bash
# Audit every runnable reftest that no expectations file lists yet, and report
# the ones that now pass without being vacuous. Report-only: no expectations
# file changes, and FAIL / VACUOUS rows do not fail the command.
#
# Usage: scripts/wpt/vacuity-sweep.sh [OUTPUT_DIR]
#   (default output: target/wpt-vacuity; WPT_SWEEP_JOBS sets --jobs)
#
# Writes into OUTPUT_DIR:
#   candidates.txt  runnable ids absent from every expectations file
#   audit.tsv       audit-vacuous-passes report, one id<TAB>VERDICT<TAB>detail row
#   keep.txt        ids with the KEEP verdict: candidates for raikiri-baseline.txt
#   meta.txt        the WPT and Raikiri revisions that were measured
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=../lib/repo_root.sh
source "$SCRIPT_DIR/../lib/repo_root.sh"

cd "$REPO_ROOT"
out_dir="${1:-$REPO_ROOT/target/wpt-vacuity}"
mkdir -p "$out_dir"

"$SCRIPT_DIR/fetch.sh"
wpt_sha="$(git -C target/wpt rev-parse HEAD)"
raikiri_sha="$(git rev-parse HEAD)"
# meta.txt names commits, so both trees must match them exactly.
require_clean() {
  if ! git -C "$1" diff --quiet HEAD --; then
    echo "$1 has uncommitted changes; the audit would not match its commit" >&2
    exit 1
  fi
}
require_clean "$REPO_ROOT"
require_clean target/wpt

"$SCRIPT_DIR/list-runnable-reftests.sh" "$out_dir/runnable.txt" target/wpt

# An id any expectations file already lists has a recorded outcome, so only
# the rest are audited. Directory prefixes in known-issues.txt are not
# subtracted: a pass inside an out-of-scope area can still become a
# regression guard (see the known-issues.txt header).
python3 - "$out_dir/runnable.txt" "$out_dir/candidates.txt" expectations <<'PY'
import sys
from pathlib import Path

runnable_path, output_path, expectations = sys.argv[1], sys.argv[2], Path(sys.argv[3])
listed = set()
for name in (
    "raikiri-baseline.txt",
    "expected-failures.txt",
    "known-issues.txt",
    "quarantine.txt",
    "deprecated.txt",
):
    for line in (expectations / name).read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        test_id = line.split("|", 1)[0].strip()
        if not test_id.endswith("/"):
            listed.add(test_id)
ids = [
    test_id
    for test_id in Path(runnable_path).read_text(encoding="utf-8").split()
    if test_id not in listed
]
Path(output_path).write_text("".join(f"{test_id}\n" for test_id in ids), encoding="utf-8")
print(f"{len(ids)} unregistered runnable reftest ids")
PY

cargo run --release --locked -p raikiri-wpt --bin audit-vacuous-passes -- \
  --wpt-root target/wpt \
  --ids "$out_dir/candidates.txt" \
  --jobs "${WPT_SWEEP_JOBS:-$(nproc)}" \
  --output "$out_dir/audit.tsv"

awk -F '\t' '$2 == "KEEP" { print $1 }' "$out_dir/audit.tsv" | sort > "$out_dir/keep.txt"
require_clean "$REPO_ROOT"
require_clean target/wpt
if [[ "$(git -C target/wpt rev-parse HEAD)" != "$wpt_sha" || "$(git rev-parse HEAD)" != "$raikiri_sha" ]]; then
  echo "a checkout moved during the audit; rerun it" >&2
  exit 1
fi
{
  echo "wpt_sha=$wpt_sha"
  echo "raikiri_sha=$raikiri_sha"
} > "$out_dir/meta.txt"
echo "$(wc -l < "$out_dir/keep.txt") KEEP ids written to $out_dir/keep.txt"
