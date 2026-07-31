#!/usr/bin/env python3
"""scripts/lib/patch_coverage.py — patch-coverage line classifier.

Called by `scripts/patch-coverage.sh` after it has already produced an lcov
report. This module owns the two pieces of logic that don't belong in shell:

  1. Parsing a `-U0` unified diff to get the exact set of *added* line
     numbers per file (gate.md §8.1.1's "merge-base 差分抽出").
  2. Deciding whether an uncovered added line is exempted by a
     `// cov:ignore: <reason>` annotation (the "escape hatch").

# The `cov:ignore` scoping rule this script implements

A `cov:ignore:` marker is either:

  - a trailing comment on a code line (`foo(); // cov:ignore: reason`) —
    exempts that line only, or
  - a comment-only line (optionally part of a multi-line comment run) —
    exempts the block of code that *immediately follows* the comment run.
    "Block" is computed by tracking `{`/`(`/`[` vs `}`/`)`/`]` depth
    (string/char literal contents and `//` line comments are stripped
    first, so brace characters inside a `"..."` don't confuse the count):
    starting at the first non-comment, non-attribute, non-blank line after
    the marker, keep including lines while cumulative depth > 0; stop
    (inclusive) once depth returns to <= 0. A single-line statement (net
    depth 0 on its own line) exempts just that one line.

This is a block-scoped, not an arm-scoped, rule: if the marker precedes an
outer statement that itself contains multiple arms (e.g. a `match` with
both a covered `Ok` arm and the one uncovered `Err` arm the comment is
actually about), every line of that outer statement is exempted, not just
the arm the author meant. This is deliberately the *safe* direction for a
false-negative/false-positive tradeoff here: an over-broad exemption only
matters if some other line in the same block is uncovered for an unrelated
reason, which `cov:ignore` additions already go through quality-lens review
for (`lens-matrix.md` §fix-application model: "cov:ignore の追加: quality
lens のみ"). New annotations should still be written as tightly as
possible (immediately before the minimal exempted line/arm) — this is a
known limitation, not a license to annotate loosely.

Lines that never appear in the lcov DA table at all (blank lines, pure
comments) are trivially exempt: absence for a normal source line means "not
an executable line, no coverage obligation". Absence for an *entire file*
(zero `SF:` records) is split into two cases, verified empirically against
this workspace's `cargo-llvm-cov 0.8.7` output rather than assumed:

  - `benches/*.rs`: a plain `cargo llvm-cov --workspace` (mirroring
    `cargo test --workspace`) never builds bench targets, so a changed
    bench file has *zero* chance of being exercised by this coverage run —
    "never instrumented" here really does mean "never ran under any gate
    check". Every added line is reported **uncovered**. This is
    intentional, not a bug: see bd raikiri-spike-iebo's gate history for
    why a bench-file diff hitting this is expected and needs either a
    `--benches`-style follow-up or a per-diff escalation.
  - `tests/*.rs` (and `examples/*.rs`): confirmed by direct measurement
    (`cargo llvm-cov -p raikiri --lcov`, then `cargo llvm-cov report
    --summary-only`) that integration-test-target source files — as
    opposed to a `#[cfg(test)] mod tests` block *inside* a `src/*.rs`
    file, which reports normally because it shares that file's SF record —
    **never get their own `SF:` record**, regardless of whether their
    tests ran and passed. `crates/raikiri/tests/external_consumer.rs` and
    `crates/raikiri/tests/parse_html_limits.rs` both ran (visible in the
    `cargo test` output) yet neither appears in the lcov export at all;
    the library functions they call *do* show up, correctly attributed to
    their own `src/*.rs` location. This is the observed *behavior*, not a
    diagnosed mechanism — this script does not know cargo-llvm-cov's
    internal reasoning for excluding these files from the report, only
    that it consistently does, on this cargo-llvm-cov version, for both
    files checked. Treat it as an empirical fact to route around, not an
    intentional-design claim to cite further. Added lines in such a file
    are reported as **unreported** (a distinct, non-failing category)
    rather than uncovered — flagging every new
    integration test as a coverage violation would be a standing false
    positive, not a signal.
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from dataclasses import dataclass, field

COMMENT_LINE_RE = re.compile(r"^\s*//")
ATTRIBUTE_LINE_RE = re.compile(r"^\s*#!?\[.*\]\s*$")
COV_IGNORE_RE = re.compile(r"cov:ignore:")
HUNK_HEADER_RE = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@")


def code_only(line: str) -> str:
    """Strip string/char literal contents and a trailing `//` comment.

    Best-effort: does not handle raw strings (`r"..."`) or byte-string
    prefixes specially, which can misparse in rare cases. Good enough for
    brace-depth bookkeeping, not a real Rust lexer.
    """
    out = []
    in_str = False
    quote = ""
    i = 0
    n = len(line)
    while i < n:
        c = line[i]
        if in_str:
            if c == "\\" and i + 1 < n:
                i += 2
                continue
            if c == quote:
                in_str = False
            i += 1
            continue
        if c in ('"', "'"):
            in_str = True
            quote = c
            i += 1
            continue
        if c == "/" and i + 1 < n and line[i + 1] == "/":
            break
        out.append(c)
        i += 1
    return "".join(out)


def brace_delta(code: str) -> int:
    opens = code.count("{") + code.count("(") + code.count("[")
    closes = code.count("}") + code.count(")") + code.count("]")
    return opens - closes


def compute_exempt_lines(lines: list[str]) -> set[int]:
    """Return the 1-indexed line numbers exempted by a `cov:ignore:` marker."""
    exempt: set[int] = set()
    pending = False
    n = len(lines)
    i = 0
    while i < n:
        raw = lines[i]
        if COMMENT_LINE_RE.match(raw):
            if COV_IGNORE_RE.search(raw):
                pending = True
            i += 1
            continue
        if not raw.strip():
            i += 1
            continue
        if ATTRIBUTE_LINE_RE.match(raw):
            # Attributes (#[cfg(...)], #[test], ...) don't themselves
            # start the exempted statement; keep looking for the real
            # first code line without losing `pending`.
            i += 1
            continue
        if pending:
            depth = 0
            j = i
            while True:
                delta = brace_delta(code_only(lines[j]))
                depth += delta
                exempt.add(j + 1)
                j += 1
                if depth <= 0 or j >= n:
                    break
            pending = False
            i = j
            continue
        i += 1

    # Trailing same-line markers exempt just their own line, independent of
    # the block scan above (a code line can't also be "pending").
    for idx, raw in enumerate(lines):
        if not COMMENT_LINE_RE.match(raw) and COV_IGNORE_RE.search(raw):
            exempt.add(idx + 1)

    return exempt


STRUCTURALLY_UNREPORTED_RE = re.compile(r"(^|/)(tests|examples)/")


def is_structurally_unreported(path: str) -> bool:
    """True for paths cargo-llvm-cov's report never lists an SF: record for,
    independent of whether their tests ran — see the module docstring's
    `tests/*.rs` case. NOT true for `benches/`, where absence instead means
    "never ran" and should fail."""
    return bool(STRUCTURALLY_UNREPORTED_RE.search(path))


@dataclass
class FileResult:
    path: str
    added_lines: list[int] = field(default_factory=list)
    uncovered: list[int] = field(default_factory=list)
    exempted: list[int] = field(default_factory=list)
    unreported: list[int] = field(default_factory=list)
    no_lcov_record: bool = False


def parse_added_lines(diff_text: str) -> dict[str, list[int]]:
    """Parse a `git diff --no-renames -U0` text into {path: [added line #]}."""
    result: dict[str, list[int]] = {}
    current_path: str | None = None
    pending_start: int | None = None
    pending_added_seen = 0

    for line in diff_text.splitlines():
        if line.startswith("+++ "):
            path = line[4:]
            if path == "/dev/null":
                current_path = None
            else:
                # "+++ b/relative/path"
                current_path = path.split("/", 1)[1] if path.startswith("b/") else path
                result.setdefault(current_path, [])
            continue
        if line.startswith("@@"):
            m = HUNK_HEADER_RE.match(line)
            if not m or current_path is None:
                pending_start = None
                continue
            start = int(m.group(1))
            count = int(m.group(2)) if m.group(2) is not None else 1
            pending_start = start
            pending_added_seen = 0
            if count == 0:
                pending_start = None  # pure deletion hunk, nothing to add
            continue
        if pending_start is not None and current_path is not None:
            if line.startswith("+") and not line.startswith("+++"):
                result[current_path].append(pending_start + pending_added_seen)
                pending_added_seen += 1
            elif line.startswith("-") and not line.startswith("---"):
                pass  # deletion, does not consume an added-line slot
            # context lines shouldn't appear under -U0, but ignore defensively
    return result


def parse_lcov(lcov_text: str, repo_root: str) -> dict[str, dict[int, int]]:
    """Parse lcov text into {repo-relative path: {line: hitcount}}."""
    files: dict[str, dict[int, int]] = {}
    current: dict[int, int] | None = None
    prefix = repo_root.rstrip("/") + "/"
    for line in lcov_text.splitlines():
        if line.startswith("SF:"):
            abs_path = line[3:]
            rel_path = abs_path[len(prefix):] if abs_path.startswith(prefix) else abs_path
            current = files.setdefault(rel_path, {})
        elif line.startswith("DA:") and current is not None:
            body = line[3:]
            line_no_str, hits_str = body.split(",", 1)
            hits_str = hits_str.split(",", 1)[0]  # some exporters add a 3rd field
            current[int(line_no_str)] = int(hits_str)
        elif line == "end_of_record":
            current = None
    return files


def git_show(repo_root: str, ref: str, path: str) -> str | None:
    proc = subprocess.run(
        ["git", "-C", repo_root, "show", f"{ref}:{path}"],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        return None
    return proc.stdout


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo-root", required=True)
    ap.add_argument("--base", required=True, help="base SHA (merge-base) diffed against HEAD")
    ap.add_argument("--head", default="HEAD")
    ap.add_argument("--lcov", required=True, help="path to lcov file")
    args = ap.parse_args()

    diff_proc = subprocess.run(
        [
            "git",
            "-C",
            args.repo_root,
            "diff",
            "--no-renames",
            "-U0",
            args.base,
            args.head,
            "--",
            "*.rs",
        ],
        capture_output=True,
        text=True,
        check=True,
    )
    added_by_file = parse_added_lines(diff_proc.stdout)

    with open(args.lcov, encoding="utf-8", errors="replace") as f:
        lcov_by_file = parse_lcov(f.read(), args.repo_root)

    results: list[FileResult] = []
    for path, added_lines in added_by_file.items():
        if not added_lines:
            continue
        fr = FileResult(path=path, added_lines=sorted(added_lines))
        head_content = git_show(args.repo_root, args.head, path)
        if head_content is None:
            # File was deleted at HEAD (shouldn't normally have added lines,
            # but be defensive) — nothing to check.
            continue
        file_lines = head_content.splitlines()
        exempt = compute_exempt_lines(file_lines)

        da_map = lcov_by_file.get(path)
        if da_map is None:
            fr.no_lcov_record = True
            if is_structurally_unreported(path):
                # tests/*.rs, examples/*.rs: cargo-llvm-cov never lists an
                # SF: record for these regardless of whether they ran — see
                # module docstring. Not a coverage failure.
                fr.unreported.extend(fr.added_lines)
            else:
                # benches/*.rs (or any other file cargo-llvm-cov's target
                # set didn't build/run at all): a real gap, same treatment
                # as an uncovered line.
                for ln in fr.added_lines:
                    if ln in exempt:
                        fr.exempted.append(ln)
                    else:
                        fr.uncovered.append(ln)
        else:
            for ln in fr.added_lines:
                if ln not in da_map:
                    continue  # not an instrumented/executable line
                if da_map[ln] > 0:
                    continue  # covered
                if ln in exempt:
                    fr.exempted.append(ln)
                else:
                    fr.uncovered.append(ln)
        results.append(fr)

    total_added = sum(len(r.added_lines) for r in results)
    total_uncovered = sum(len(r.uncovered) for r in results)
    total_exempted = sum(len(r.exempted) for r in results)
    total_unreported = sum(len(r.unreported) for r in results)

    print(f"patch coverage: base={args.base} head={args.head}")
    print(f"changed .rs files with added lines: {len(results)}")
    print(f"total added lines (diff): {total_added}")
    print(f"total cov:ignore-exempted added lines: {total_exempted}")
    print(f"total unreported added lines (tests/examples, informational): {total_unreported}")
    print(f"total uncovered added lines (FAIL if > 0): {total_uncovered}")
    print()

    if total_unreported:
        print("Unreported changed lines (file:line) — cargo-llvm-cov does not")
        print("list an SF: record for tests/*.rs or examples/*.rs regardless of")
        print("whether the test ran; not counted toward PASS/FAIL:")
        for r in results:
            for ln in r.unreported:
                print(f"  {r.path}:{ln}")
        print()

    if total_uncovered == 0:
        print("PASS: all changed lines are covered or cov:ignore-exempted.")
        return 0

    print("Uncovered changed lines (file:line):")
    for r in results:
        if not r.uncovered:
            continue
        note = " [no lcov record for this file — not built under the coverage run, e.g. a bench target]" if r.no_lcov_record else ""
        for ln in r.uncovered:
            print(f"  {r.path}:{ln}{note}")
    print()
    print("Per gate.md §8.1.1: either add a covering test (task scope) or")
    print("escalate to a bd issue (out-of-scope) for each line above, or add")
    print("`// cov:ignore: <reason>` if it is a defensible untestable branch.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
