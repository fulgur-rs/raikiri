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

python3 "$SCRIPT_DIR/survey_reftests.py" --wpt-root target/wpt --format json \
  --output "$work_dir/survey.json"
python3 - "$work_dir/survey.json" "$work_dir/runnable.txt" target/wpt <<'PY'
import html.parser
import json
import sys
from pathlib import Path


class VariantParser(html.parser.HTMLParser):
    """Collect `<meta name="variant" content="?..." | "#...">` suffixes."""

    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.variants: list[str] = []

    def handle_starttag(self, tag, attrs):
        if tag.casefold() != "meta":
            return
        values = {key.casefold(): value or "" for key, value in attrs}
        content = values.get("content", "")
        if values.get("name", "").casefold() == "variant" and content[:1] in ("?", "#") and len(content) > 1:
            self.variants.append(content)

    handle_startendtag = handle_starttag


report = json.load(open(sys.argv[1], encoding="utf-8"))
wpt_root = Path(sys.argv[3])
ids = []
for category in report["categories"].values():
    for theme in category["themes"]:
        for test in theme["tests"]:
            if not test["structurally_runnable"]:
                continue
            test_id = test["test_id"]
            parser = VariantParser()
            parser.feed((wpt_root / test_id).read_bytes().decode("utf-8", errors="replace"))
            # A file with declared variants runs once per variant, as WPT does;
            # the dashboard reduces them back to one outcome per file. The
            # report runner does not apply `#` variants yet and reports them as
            # ERROR, which keeps such a file from counting as PASS.
            ids.extend(f"{test_id}{variant}" for variant in parser.variants or [""])
ids.sort()
with open(sys.argv[2], "w", encoding="utf-8") as out:
    out.write("".join(f"{test_id}\n" for test_id in ids))
print(f"{len(ids)} runnable reftest ids (variants expanded)")
PY

cargo run --release --locked -p raikiri-wpt --bin run-baseline-report -- \
  --wpt-root target/wpt \
  --baseline "$work_dir/runnable.txt" \
  --jobs "${WPT_SWEEP_JOBS:-$(nproc)}" \
  --output "$output"
# Record what was measured so the dashboard can refuse a sweep from another
# WPT pin and name the Raikiri revision it reflects.
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
