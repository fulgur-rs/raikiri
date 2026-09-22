#!/usr/bin/env python3
"""Survey WPT reftests in a checkout without running or pinning them.

The inventory is grouped by the first directory below each WPT category. This
is a navigation aid, not proof that a group has one semantic theme or that a
case can render successfully. Root-level cases use a conservative filename
prefix and are marked for manual theme review. This command never changes an
expectations file.
"""
from __future__ import annotations

import argparse
import codecs
import html.parser
import json
import os
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path
from urllib.parse import unquote, urlsplit

# Match the extensions currently discovered by raikiri-wpt, plus WPT formats
# that the survey should expose as unsupported candidates rather than omit.
HTML_EXTENSIONS = {".html", ".htm", ".xhtml", ".xht", ".xml", ".svg"}
RUNNER_EXTENSIONS = {".html", ".htm", ".xhtml"}
DEFAULT_ROOT = Path("target/wpt")
DEFAULT_BASELINE = Path("expectations/raikiri-baseline.txt")
XML_ENCODING_RE = re.compile(rb"<\?xml\b[^>]*\bencoding\s*=\s*(['\"])([^'\"]+)\1", re.IGNORECASE)
META_CHARSET_RE = re.compile(rb"<meta\b[^>]*\bcharset\s*=\s*['\"]?\s*([^\s/;'\"]+)", re.IGNORECASE)
META_CONTENT_ENCODING_RE = re.compile(
    rb"<meta\b[^>]*\bcontent\s*=\s*['\"][^'\"]*?charset\s*=\s*([^\s;/'\"]+)",
    re.IGNORECASE,
)


def decode_markup(data: bytes) -> str:
    """Decode WPT markup using its BOM or declared XML/HTML charset."""
    if data.startswith((codecs.BOM_UTF32_LE, codecs.BOM_UTF32_BE)):
        return data.decode("utf-32", errors="replace")
    if data.startswith(codecs.BOM_UTF8):
        return data.decode("utf-8-sig", errors="replace")
    if data.startswith((codecs.BOM_UTF16_LE, codecs.BOM_UTF16_BE)):
        return data.decode("utf-16", errors="replace")

    prefix = data[:4096]
    for pattern in (XML_ENCODING_RE, META_CHARSET_RE, META_CONTENT_ENCODING_RE):
        match = pattern.search(prefix)
        if match:
            try:
                encoding_group = 2 if pattern is XML_ENCODING_RE else 1
                encoding = match.group(encoding_group).decode("ascii")
                return data.decode(encoding, errors="replace")
            except (LookupError, UnicodeDecodeError):
                pass
    # Reftest syntax uses ASCII markup, so replacement preserves tags and hrefs
    # even when an unusual fixture omits a useful encoding declaration.
    return data.decode("utf-8", errors="replace")


class ReftestLinkParser(html.parser.HTMLParser):
    """Collect WPT match and mismatch links from HTML-like files."""

    def __init__(self) -> None:
        super().__init__(convert_charrefs=True)
        self.links: list[dict[str, str]] = []

    def handle_starttag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        if tag.casefold() != "link":
            return
        values = {key.casefold(): value for key, value in attrs}
        rel = values.get("rel") or ""
        href = values.get("href") or ""
        for relation in rel.casefold().split():
            if relation in {"match", "mismatch"}:
                self.links.append({"kind": relation, "href": href})

    def handle_startendtag(self, tag: str, attrs: list[tuple[str, str | None]]) -> None:
        self.handle_starttag(tag, attrs)


def read_baseline(path: Path) -> set[str]:
    if not path.is_file():
        raise ValueError(f"baseline file not found: {path}")
    return {
        line.strip()
        for line in path.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    }


def checkout_revision(root: Path) -> str:
    """Return a revision only when root is itself the WPT Git worktree."""
    root = root.resolve()
    try:
        result = subprocess.run(
            ["git", "-C", str(root), "rev-parse", "--show-toplevel", "HEAD"],
            check=True,
            capture_output=True,
            text=True,
            timeout=5,
        )
    except (OSError, subprocess.SubprocessError):
        return "unknown"

    lines = result.stdout.strip().splitlines()
    if len(lines) != 2 or Path(lines[0]).resolve() != root:
        # `git -C <directory>` walks up to a parent repository. Do not label a
        # copied WPT subtree with the Raikiri checkout's unrelated commit.
        return "unknown"
    return lines[1].strip()


def output_overwrites_baseline(output_path: Path, baseline_path: Path) -> bool:
    """Detect direct, symlink, and hard-link aliases before writing a report."""
    if output_path.resolve() == baseline_path.resolve():
        return True
    try:
        return output_path.exists() and baseline_path.exists() and os.path.samefile(output_path, baseline_path)
    except OSError:
        return False


def resolve_reference(root: Path, test_path: Path, href: str) -> dict[str, str]:
    """Classify one reference href without fetching external resources."""
    if not href.strip():
        return {"href": href, "status": "empty-href", "reference_id": ""}

    parsed = urlsplit(href)
    reference_path = unquote(parsed.path)
    if parsed.scheme or parsed.netloc:
        return {"href": href, "status": "external", "reference_id": ""}

    if not reference_path:
        candidate = test_path
    elif reference_path.startswith("/"):
        candidate = root / reference_path.lstrip("/")
    else:
        candidate = test_path.parent / reference_path
    resolved_root = root.resolve()
    resolved_candidate = candidate.resolve()
    try:
        relative = resolved_candidate.relative_to(resolved_root).as_posix()
    except ValueError:
        return {"href": href, "status": "outside-root", "reference_id": ""}

    if not resolved_candidate.is_file():
        return {"href": href, "status": "missing", "reference_id": relative}
    return {"href": href, "status": "local", "reference_id": relative}


def category_and_theme(test_id: str) -> tuple[str, str, str]:
    parts = Path(test_id).parts
    directories = parts[:-1]
    if len(directories) >= 2:
        category = "/".join(directories[:2])
        theme_directories = directories[2:]
    elif directories:
        category = directories[0]
        theme_directories = ()
    else:
        category = "(root)"
        theme_directories = ()

    if theme_directories:
        return category, theme_directories[0], "directory"

    # WPT filenames often use numbered cases for a single feature. Strip only
    # a terminal numeric case suffix; retain an explicit review marker because
    # naming conventions are not a reliable semantic classifier.
    stem = Path(parts[-1]).stem
    theme_stem = re.sub(r"[-_]\d+[a-z]?$", "", stem, flags=re.IGNORECASE) or stem
    return category, f"(root files)/{theme_stem}", "filename-prefix"


def sparse_checkout_patterns(root: Path) -> list[str] | None:
    sparse_file = root / ".git" / "info" / "sparse-checkout"
    if not sparse_file.is_file():
        return None
    return [
        line.strip()
        for line in sparse_file.read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.lstrip().startswith("#")
    ]


def tracked_wpt_paths(root: Path) -> set[str] | None:
    """List indexed paths only when root is the top-level WPT Git checkout."""
    try:
        top_level = subprocess.run(
            ["git", "-C", str(root), "rev-parse", "--show-toplevel"],
            check=True,
            capture_output=True,
            text=True,
            timeout=5,
        )
    except (OSError, subprocess.SubprocessError):
        return None
    if Path(top_level.stdout.strip()).resolve() != root.resolve():
        return None

    try:
        result = subprocess.run(
            ["git", "-C", str(root), "ls-files", "--cached", "--full-name", "-z"],
            check=True,
            capture_output=True,
            timeout=10,
        )
    except (OSError, subprocess.SubprocessError) as error:
        raise ValueError(f"could not list tracked WPT paths: {error}") from error
    return {os.fsdecode(path) for path in result.stdout.split(b"\0") if path}


def scan_reftests(
    root: Path,
    baseline: set[str],
    category_filter: str | None = None,
    theme_filter: str | None = None,
) -> tuple[list[dict], list[str]]:
    if not root.is_dir():
        raise ValueError(f"WPT root not found: {root}")

    root = root.resolve()
    errors: list[str] = []
    prefix = category_filter.strip("/") if category_filter else None
    indexed_paths = tracked_wpt_paths(root)
    if indexed_paths is not None:
        # Ignore local untracked fixtures so the inventory matches the pinned
        # WPT checkout rather than leftovers from another task.
        test_files = [
            root / relative
            for relative in sorted(indexed_paths)
            if (not prefix or relative == prefix or relative.startswith(prefix + "/"))
            and Path(relative).suffix.casefold() in HTML_EXTENSIONS
            and (root / relative).is_file()
        ]
    else:
        test_files = []

        def on_walk_error(error: OSError) -> None:
            errors.append(f"scan error: {error}")

        for directory, names, filenames in os.walk(root, onerror=on_walk_error):
            relative_directory = Path(directory).relative_to(root).as_posix()
            names[:] = sorted(name for name in names if name != ".git")
            if category_filter:
                normalized = category_filter.strip("/")
                directory_id = "" if relative_directory == "." else relative_directory
                relevant = (
                    not directory_id
                    or directory_id == normalized
                    or directory_id.startswith(normalized + "/")
                    or normalized.startswith(directory_id + "/")
                )
                if not relevant:
                    names[:] = []
                    continue
            for filename in sorted(filenames):
                path = Path(directory) / filename
                if path.suffix.casefold() in HTML_EXTENSIONS:
                    test_files.append(path)

    entries: list[dict] = []
    for path in sorted(test_files):
        test_id = path.relative_to(root).as_posix()
        category, theme, theme_source = category_and_theme(test_id)
        if prefix and not (test_id == prefix or test_id.startswith(prefix + "/")):
            continue
        if theme_filter and theme != theme_filter:
            continue
        try:
            source = decode_markup(path.read_bytes())
        except OSError as error:
            errors.append(f"could not read {test_id}: {error}")
            continue

        parser = ReftestLinkParser()
        try:
            parser.feed(source)
            parser.close()
        except Exception as error:  # HTMLParser should be tolerant; retain diagnostics if not.
            errors.append(f"could not parse {test_id}: {error}")
            continue
        if not parser.links:
            continue

        references = [resolve_reference(root, path, link["href"]) | {"kind": link["kind"]} for link in parser.links]
        baseline_member = test_id in baseline
        unsupported = path.suffix.casefold() not in RUNNER_EXTENSIONS
        blockers = sorted({
            reference["status"]
            for reference in references
            if reference["status"] not in {"local"}
        })
        if unsupported:
            blockers.append("runner-unsupported-extension")
        entries.append({
            "test_id": test_id,
            "category": category,
            "theme": theme,
            "theme_source": theme_source,
            "theme_needs_review": True,
            "baseline": baseline_member,
            "pair_count": len(references),
            "references": references,
            "runner_supported_extension": not unsupported,
            "structurally_runnable": not blockers,
            "blockers": sorted(set(blockers)),
        })

    return entries, errors


def make_report(
    root: Path,
    baseline_path: Path,
    entries: list[dict],
    errors: list[str],
    category_filter: str | None,
    theme_filter: str | None,
) -> dict:
    sparse_patterns = sparse_checkout_patterns(root)
    groups: dict[tuple[str, str], list[dict]] = defaultdict(list)
    for entry in entries:
        groups[(entry["category"], entry["theme"])].append(entry)

    categories: dict[str, dict] = {}
    for (category, theme), tests in sorted(groups.items()):
        category_data = categories.setdefault(category, {"test_files": 0, "pairs": 0, "baselined": 0, "themes": []})
        theme_data = {
            "theme": theme,
            "theme_source": tests[0]["theme_source"],
            "theme_needs_review": any(test["theme_needs_review"] for test in tests),
            "test_files": len(tests),
            "pairs": sum(test["pair_count"] for test in tests),
            "baselined": sum(test["baseline"] for test in tests),
            "unbaselined": sum(not test["baseline"] for test in tests),
            "structurally_runnable_unbaselined": sum(
                not test["baseline"] and test["structurally_runnable"] for test in tests
            ),
            "tests": tests,
        }
        category_data["test_files"] += theme_data["test_files"]
        category_data["pairs"] += theme_data["pairs"]
        category_data["baselined"] += theme_data["baselined"]
        category_data["themes"].append(theme_data)

    return {
        "schema_version": 1,
        "wpt_root": str(root),
        "wpt_revision": checkout_revision(root),
        "checkout_scope": "materialized sparse checkout" if sparse_patterns is not None else "materialized checkout",
        "sparse_checkout_patterns": sparse_patterns,
        "viewport_css_px": [800, 600],
        "baseline_path": str(baseline_path),
        "category_filter": category_filter,
        "theme_filter": theme_filter,
        "summary": {
            "categories": len(categories),
            "test_files_with_reftest_links": len(entries),
            "reference_pairs": sum(entry["pair_count"] for entry in entries),
            "baselined_test_files": sum(entry["baseline"] for entry in entries),
            "unbaselined_test_files": sum(not entry["baseline"] for entry in entries),
            "structurally_runnable_unbaselined": sum(
                not entry["baseline"] and entry["structurally_runnable"] for entry in entries
            ),
            "scan_errors": len(errors),
        },
        "categories": categories,
        "scan_errors": errors,
        "note": (
            "Inventory only: baseline membership is not a fresh PASS result. "
            "A test file with multiple reference links must pass every pair before it is promoted. "
            "Theme grouping and structural runnability require review. Sparse checkout scope is reported separately."
        ),
    }


def render_text(report: dict, list_tests: bool) -> str:
    summary = report["summary"]
    lines = [
        "WPT reftest survey (inventory only; no tests executed; baseline unchanged)",
        f"WPT revision: {report['wpt_revision']}",
        f"Checkout scope: {report['checkout_scope']}"
        + (f" ({len(report['sparse_checkout_patterns'])} sparse patterns)" if report["sparse_checkout_patterns"] is not None else ""),
        f"Viewport: {report['viewport_css_px'][0]}x{report['viewport_css_px'][1]} CSS px",
        f"Filter: category={report['category_filter'] or '*'}, theme={report['theme_filter'] or '*'}",
        "Counts: "
        f"{summary['test_files_with_reftest_links']} test files / "
        f"{summary['reference_pairs']} reference pairs; "
        f"{summary['baselined_test_files']} baseline / "
        f"{summary['unbaselined_test_files']} unbaselined / "
        f"{summary['structurally_runnable_unbaselined']} structurally runnable candidates",
        "",
    ]
    for category, category_data in report["categories"].items():
        lines.append(
            f"{category}: {category_data['test_files']} files, {category_data['pairs']} pairs, "
            f"{category_data['baselined']} baseline"
        )
        for theme in category_data["themes"]:
            review = f" [review {theme['theme_source']} grouping]"
            lines.append(
                f"  {theme['theme']}: {theme['test_files']} files / {theme['pairs']} pairs; "
                f"{theme['baselined']} baseline, {theme['unbaselined']} unbaselined, "
                f"{theme['structurally_runnable_unbaselined']} structurally runnable{review}"
            )
            if list_tests:
                for test in theme["tests"]:
                    state = "baseline" if test["baseline"] else "candidate"
                    blockers = f" blockers={','.join(test['blockers'])}" if test["blockers"] else ""
                    lines.append(f"    {state}: {test['test_id']} ({test['pair_count']} pairs){blockers}")
    if report["scan_errors"]:
        lines.extend(["", "Scan errors:", *(f"  - {error}" for error in report["scan_errors"])])
    if report["sparse_checkout_patterns"] is not None:
        lines.append("Sparse checkout: categories not materialized in this checkout are absent, not empty.")
    lines.extend(["", report["note"]])
    return "\n".join(lines) + "\n"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Survey WPT reftests by category and theme")
    parser.add_argument("--wpt-root", type=Path, default=DEFAULT_ROOT)
    parser.add_argument("--baseline", type=Path, default=DEFAULT_BASELINE)
    parser.add_argument("--category", help="only include test IDs under this WPT path prefix")
    parser.add_argument("--theme", help="only include the exact survey theme label")
    parser.add_argument("--format", choices=("text", "json"), default="text")
    parser.add_argument("--output", type=Path, help="write the report here instead of stdout")
    parser.add_argument("--list-tests", action="store_true", help="include each test ID in text output")
    args = parser.parse_args(argv)

    if args.output and output_overwrites_baseline(args.output, args.baseline):
        print("error: survey output must not overwrite the baseline", file=sys.stderr)
        return 2

    try:
        baseline = read_baseline(args.baseline)
        entries, errors = scan_reftests(args.wpt_root, baseline, args.category, args.theme)
        report = make_report(args.wpt_root, args.baseline, entries, errors, args.category, args.theme)
    except (OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2

    output = json.dumps(report, indent=2, ensure_ascii=False) + "\n" if args.format == "json" else render_text(report, args.list_tests)
    try:
        if args.output:
            args.output.parent.mkdir(parents=True, exist_ok=True)
            args.output.write_text(output, encoding="utf-8")
            print(f"survey written to {args.output}")
        else:
            sys.stdout.write(output)
    except OSError as error:
        print(f"error: could not write report: {error}", file=sys.stderr)
        return 2
    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
