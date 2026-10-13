#!/usr/bin/env bash
# Write the ids of every structurally runnable WPT reftest, one per line, with
# declared `<meta name="variant">` suffixes expanded as WPT runs them.
#
# Usage: scripts/wpt/list-runnable-reftests.sh OUTPUT.txt [WPT_ROOT]
#   (WPT_ROOT defaults to target/wpt; the survey JSON is written next to
#   OUTPUT.txt as survey.json)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
output="${1:?usage: list-runnable-reftests.sh OUTPUT.txt [WPT_ROOT]}"
wpt_root="${2:-target/wpt}"
work_dir="$(dirname "$output")"
mkdir -p "$work_dir"

python3 "$SCRIPT_DIR/survey_reftests.py" --wpt-root "$wpt_root" --format json \
  --output "$work_dir/survey.json"
python3 - "$work_dir/survey.json" "$output" "$wpt_root" <<'PY'
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
