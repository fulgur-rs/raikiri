#!/usr/bin/env python3
"""Apply recorded AI-agent decisions from a meta-assert review.

The default mode is a report-only dry run. Baseline changes require --apply.
Each review row is JSONL with at least:

    {"test_id":"css/foo.html", "decision":"pass", "reason":"..."}

Allowed decisions are pass, fail, needs-human, and not-renderable.
"""
from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

DECISIONS = {"pass", "fail", "needs-human", "not-renderable"}


def load_jsonl(path: Path) -> list[dict]:
    rows = []
    for line_no, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        if not raw.strip():
            continue
        try:
            row = json.loads(raw)
        except json.JSONDecodeError as exc:
            raise ValueError(f"{path}:{line_no}: invalid JSON: {exc}") from exc
        if not isinstance(row, dict):
            raise ValueError(f"{path}:{line_no}: expected a JSON object")
        rows.append(row)
    return rows


def main() -> int:
    parser = argparse.ArgumentParser(description="Apply meta-assert agent review records")
    parser.add_argument("--manifest", required=True, type=Path)
    parser.add_argument("--reviews", required=True, type=Path)
    parser.add_argument("--baseline", default=Path("expectations/meta-assert-baseline.txt"), type=Path)
    parser.add_argument("--apply", action="store_true", help="append accepted pass rows to baseline")
    args = parser.parse_args()

    for path, label in ((args.manifest, "manifest"), (args.reviews, "reviews"), (args.baseline, "baseline")):
        if not path.is_file():
            if label == "reviews":
                print(
                    f"error: review file not found: {path}\n"
                    "create it from reviews.template.jsonl after the agent has filled each decision",
                    file=sys.stderr,
                )
            else:
                print(f"error: {label} file not found: {path}", file=sys.stderr)
            return 2

    try:
        manifest_rows = load_jsonl(args.manifest)
        review_rows = load_jsonl(args.reviews)
    except (OSError, ValueError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2

    manifest = {row.get("test_id"): row for row in manifest_rows if row.get("test_id")}
    baseline_lines = args.baseline.read_text(encoding="utf-8").splitlines()
    baseline = {line.strip() for line in baseline_lines if line.strip() and not line.lstrip().startswith("#")}
    accepted: list[str] = []
    seen_reviews: set[str] = set()
    errors: list[str] = []

    for index, row in enumerate(review_rows, 1):
        test_id = row.get("test_id")
        decision = row.get("decision")
        if not isinstance(test_id, str) or not test_id:
            errors.append(f"review {index}: missing test_id")
            continue
        if test_id in seen_reviews:
            errors.append(f"review {index}: duplicate test_id {test_id}")
            continue
        seen_reviews.add(test_id)
        if test_id not in manifest:
            errors.append(f"review {index}: {test_id} is not in manifest")
            continue
        if decision not in DECISIONS:
            errors.append(f"review {index}: invalid decision {decision!r}")
            continue
        if decision == "pass" and manifest[test_id].get("status") != "pending":
            errors.append(f"review {index}: {test_id} is not renderable")
            continue
        if decision == "pass" and test_id not in baseline:
            accepted.append(test_id)

    if errors:
        for error in errors:
            print(f"error: {error}", file=sys.stderr)
        return 2

    print(f"reviews: {len(review_rows)}, accepted pass rows: {len(accepted)}")
    if not args.apply:
        print("dry-run: baseline unchanged (use --apply to append accepted rows)")
        return 0

    if accepted:
        with args.baseline.open("a", encoding="utf-8") as output:
            for test_id in accepted:
                output.write(f"{test_id}\n")
    print(f"appended {len(accepted)} baseline entries to {args.baseline}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
