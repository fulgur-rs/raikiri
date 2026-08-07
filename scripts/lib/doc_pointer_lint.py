#!/usr/bin/env python3
"""scripts/lib/doc_pointer_lint.py — `crate::…` doc-pointer census / lint.

Shared checker for three sibling bd issues that all reduce to the same
occurrence-level census over `crates/*/src/**/*.rs` (bd raikiri-spike-acsw /
raikiri-spike-vyse / raikiri-spike-luxp all say explicitly: don't design
three separate checks). It classifies every `//`-comment line as `doc`
(`///` / `//!`) or `plain` (any other `//` line, including `////`-style
separators) and extracts two kinds of Markdown-link-shaped occurrence:

  - "linked" — bracket-wraps-backtick or bare-bracket form, e.g.
    `` [`crate::foo::Bar`] `` or `[crate::foo::Bar]`. This is what
    `rustdoc::broken_intra_doc_links` actually resolves (only in `doc`
    lines — rustdoc never reads a `plain` `//` comment at all).
  - "bare" — a plain backtick code span, e.g. `` `crate::foo::Bar` ``, not
    wrapped in `[...]`.

Two gate roles, both occurrence-counted, both restricted to `crate::…`
pointers (module-relative pointers are a known under-count — see AGENTS.md's
`crate::…` pointer section on why `crate::`-only grep already under-counts
by design; this script inherits that same scope rather than widening it,
to keep its numbers comparable to the historical census this task is
retiring):

  1. **vyse rule (hard-zero)**: bracket-link syntax in a `plain` `//`
     comment line. AGENTS.md's "既知の限界" section says rustdoc reads zero
     bytes of a plain `//` comment, so a bracket there *looks* like a
     verified intra-doc link but is not — worse than a bare backtick span,
     which at least doesn't claim to be verified. This applies to ANY
     bracket-link, not just `crate::`-prefixed ones (bd raikiri-spike-vyse
     comment 2026-07-28: scoping this to `crate::` only closes 3/21 sites;
     the danger is unrelated to the `crate::` prefix). Must be 0.

     Role 1 has **no ignore-marker escape hatch** — unlike role 2, a
     flagged occurrence cannot be exempted with
     `doc-pointer-lint:ignore:`. AGENTS.md's decided rule for plain `//`
     comments is unconditional ("一切書かない"), so the only remedy is to
     rewrite the line as a bare code span. This was checked for false
     positives at authoring time: `_BARE_BRACKET_RE`'s shape would also
     match a Markdown *inline* link `[text](url)`, which is a different
     (and legitimate) construct rustdoc also doesn't touch outside a doc
     comment — a repo-wide grep for that shape in `crates/*/src` plain
     comments found 0 occurrences, and block comments (`/* */`) — which
     this script does not scan at all — were independently confirmed to
     hold 0 `crate::` references (bd raikiri-spike-vyse's original
     census). If either shape is ever introduced, `_BARE_BRACKET_RE`
     would misfire on the former; re-check before trusting role 1's
     output blindly on a codebase that has grown either construct.

  2. **luxp ratchet (drift prevention)**: bare (non-linked) `crate::…`
     pointers in `doc` (`///` / `//!`) comment lines — i.e. occurrences of
     the exact violation AGENTS.md's "`crate::…` pointer は intra-doc link
     で書く" rule prohibits, that have not yet been fixed. Must not exceed
     a pinned baseline (`doc_pointer_lint_baseline.txt`).

Opt-out classification (AGENTS.md "opt-out (link 化しない)"), applied only
to the role-2 (ratchet) candidate set:

  1. **(removed) marker** — grep-able, same line as the flagged span.
  2. **points into `#[cfg(test)] mod tests`** — approximated by checking
     whether the pointer path itself contains a `tests::` segment (AGENTS.md
     opt-out 2's own reasoning: "path 中の `tests::` がtest であることを
     示す"), per bd raikiri-spike-luxp's own framing of this opt-out as
     "path の `tests::` パターンで概ね判定可能".
  3. **rustdoc-blind position** (`#[test]` item doc, fn-body-local item
     doc, `#[doc(hidden)]`, `tests/`/`benches`/`examples` targets,
     `#[cfg]`-excluded item, unexpanded `macro_rules!` body) — bd
     raikiri-spike-luxp's issue text states plainly that this is **not
     statically decidable** from the surrounding text alone. This script
     does not attempt to decide it: such spans are counted toward the
     ratchet by default (fail-closed), and an author who has separately
     confirmed (by the "わざと壊して確かめる" procedure in AGENTS.md) that a
     specific span is rustdoc-blind must say so explicitly with a
     `doc-pointer-lint:ignore: <reason>` marker on the same line — the same
     "explicit opt-out only, no implicit exemption" posture
     `scripts/lib/patch_coverage.py`'s `cov:ignore:` uses. Unlike
     `cov:ignore:`, this marker is same-line only (no block-scoping): a doc
     comment span doesn't have a brace-delimited "block" to scope over the
     way a statement does, so there is nothing for a preceding-line marker
     to unambiguously bound.

Not in scope for this script (see the 3 issues' text for why): gate §8.1 /
scripts/gate.sh integration, lowering the baseline, and any check on
non-`crate::` pointers.
"""

from __future__ import annotations

import argparse
import glob
import re
import sys
from dataclasses import dataclass, field
from pathlib import Path

# -- line classification ---------------------------------------------------

# `///x` is a doc comment (no space required); `////...` is NOT — rustc
# treats 4-or-more leading slashes as a normal (non-doc) comment, commonly
# used as a section-separator idiom. `//!` doc comments have no equivalent
# ambiguity.
_DOC_OUTER_RE = re.compile(r"^///(?!/)")
_DOC_INNER_RE = re.compile(r"^//!")


def classify_line(lstripped: str) -> str:
    """Classify an already-lstripped line as 'doc', 'plain', or 'code'."""
    if _DOC_OUTER_RE.match(lstripped) or _DOC_INNER_RE.match(lstripped):
        return "doc"
    if lstripped.startswith("//"):
        return "plain"
    return "code"


# -- occurrence extraction --------------------------------------------------

# Step A: bracket wrapping a single backtick span — the canonical intra-doc
# link-with-code-span form, e.g. `` [`crate::foo::Bar`] ``.
_BRACKET_BACKTICK_RE = re.compile(r"\[`([^`\]\n]+)`\]")
# Step C (run against what step A left behind): any remaining backtick span.
_BACKTICK_RE = re.compile(r"`([^`\n]+)`")
# Step E (run against what steps A+C left behind, so nothing already
# consumed as a code span — e.g. `` `&[Declaration]` `` — can re-match
# here): a bare bracket-link, e.g. `[crate::foo::Bar]` or `[PropertyKey]`.
# Excludes a `[` immediately preceded by `#` (attribute, `#[cfg(test)]`) or
# `&` (slice type written without backticks, `&[T]`).
_BARE_BRACKET_RE = re.compile(r"(?<![#&])\[([A-Za-z_][A-Za-z0-9_:<>]*(?:\(\))?)\]")

_IGNORE_MARKER_RE = re.compile(r"doc-pointer-lint:ignore:\s*(\S.*)$")


@dataclass
class LineFindings:
    linked_spans: list[str] = field(default_factory=list)  # from [`...`] or bare [Ident]
    bare_backtick_spans: list[str] = field(default_factory=list)  # remaining `...`


def analyze_line(raw: str) -> LineFindings:
    """Extract link-shaped and bare-code-span occurrences from one line.

    Order matters: each step consumes (blanks out) what it matched before
    the next step runs, so a backtick span already claimed by the
    bracket-wrapping-backtick form isn't re-counted as a bare backtick span,
    and neither of those can leave stray brackets for the bare-bracket step
    to misfire on (e.g. `` `&[Declaration]` `` is fully consumed by step C
    before step E ever sees it).
    """
    findings = LineFindings()
    work = raw

    def _consume_linked(m: re.Match[str]) -> str:
        findings.linked_spans.append(m.group(1))
        return " " * len(m.group(0))

    work = _BRACKET_BACKTICK_RE.sub(_consume_linked, work)

    def _consume_backtick(m: re.Match[str]) -> str:
        findings.bare_backtick_spans.append(m.group(1))
        return " " * len(m.group(0))

    work = _BACKTICK_RE.sub(_consume_backtick, work)

    for m in _BARE_BRACKET_RE.finditer(work):
        findings.linked_spans.append(m.group(1))

    return findings


# -- per-occurrence records ---------------------------------------------------


@dataclass
class Occurrence:
    path: str
    line: int
    span: str
    raw_line: str


@dataclass
class CensusResult:
    # Role 1 (vyse): ANY bracket-link in a `plain` line, crate:: or not.
    plain_bracket_violations: list[Occurrence] = field(default_factory=list)
    # Role 2 (luxp) candidate set, pre-opt-out: bare `crate::` backtick spans
    # in `doc` lines.
    doc_bare_crate_all: list[Occurrence] = field(default_factory=list)
    # Subset of the above that survives opt-out classification — this is
    # the number compared against the pinned baseline.
    doc_bare_crate_ratchet: list[Occurrence] = field(default_factory=list)
    doc_bare_crate_excluded: list[tuple[Occurrence, str]] = field(default_factory=list)
    # Informational only, not gated:
    doc_linked_crate_count: int = 0
    plain_bare_crate_count: int = 0


def _classify_opt_out(occ: Occurrence) -> str | None:
    """Return the opt-out reason name if `occ` is exempted from the role-2
    ratchet, else None (counted, fail-closed default)."""
    if "(removed)" in occ.raw_line:
        return "opt-out-1:(removed)"
    if "tests::" in occ.span:
        return "opt-out-2:tests::"
    if _IGNORE_MARKER_RE.search(occ.raw_line):
        return "explicit:doc-pointer-lint:ignore:"
    return None


def census_file(path: str, text: str) -> CensusResult:
    result = CensusResult()
    for lineno, raw in enumerate(text.splitlines(), start=1):
        lstripped = raw.lstrip()
        kind = classify_line(lstripped)
        if kind == "code":
            continue
        findings = analyze_line(raw)
        if kind == "plain":
            for span in findings.linked_spans:
                result.plain_bracket_violations.append(Occurrence(path, lineno, span, raw))
            for span in findings.bare_backtick_spans:
                if span.startswith("crate::"):
                    result.plain_bare_crate_count += 1
        else:  # kind == "doc"
            for span in findings.linked_spans:
                if span.startswith("crate::"):
                    result.doc_linked_crate_count += 1
            for span in findings.bare_backtick_spans:
                if span.startswith("crate::"):
                    occ = Occurrence(path, lineno, span, raw)
                    result.doc_bare_crate_all.append(occ)
                    reason = _classify_opt_out(occ)
                    if reason is None:
                        result.doc_bare_crate_ratchet.append(occ)
                    else:
                        result.doc_bare_crate_excluded.append((occ, reason))
    return result


def merge(results: list[CensusResult]) -> CensusResult:
    merged = CensusResult()
    for r in results:
        merged.plain_bracket_violations.extend(r.plain_bracket_violations)
        merged.doc_bare_crate_all.extend(r.doc_bare_crate_all)
        merged.doc_bare_crate_ratchet.extend(r.doc_bare_crate_ratchet)
        merged.doc_bare_crate_excluded.extend(r.doc_bare_crate_excluded)
        merged.doc_linked_crate_count += r.doc_linked_crate_count
        merged.plain_bare_crate_count += r.plain_bare_crate_count
    return merged


def discover_files(repo_root: str) -> list[str]:
    """`crates/*/src/**/*.rs`, repo-relative, sorted for stable output.

    Recursive under each crate's `src/` (not just the top level) so nested
    module dirs (`crates/raikiri-html/src/ua/`, `crates/raikiri-traits/
    src/page/`, `crates/raikiri-wpt/src/bin/`, `.../snapshots/`) are
    covered — a flat `crates/*/src/*.rs` glob (the literal pattern quoted
    in the bd raikiri-spike-vyse/luxp issue text) would silently skip
    those.
    """
    root = Path(repo_root)
    pattern = str(root / "crates" / "*" / "src" / "**" / "*.rs")
    paths = sorted(glob.glob(pattern, recursive=True))
    return [str(Path(p).relative_to(root)) for p in paths]


def run_census(repo_root: str) -> CensusResult:
    results = []
    for rel_path in discover_files(repo_root):
        text = Path(repo_root, rel_path).read_text(encoding="utf-8")
        results.append(census_file(rel_path, text))
    return merge(results)


def load_baseline(baseline_file: str) -> int:
    for line in Path(baseline_file).read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        return int(line)
    raise ValueError(f"{baseline_file}: no non-comment integer line found")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--repo-root", required=True)
    ap.add_argument(
        "--baseline-file",
        default=None,
        help="path to the pinned ratchet baseline (default: "
        "<this script's dir>/doc_pointer_lint_baseline.txt)",
    )
    ap.add_argument(
        "--print-count",
        action="store_true",
        help="print only the current role-2 ratchet-relevant count and exit "
        "0 (for regenerating the baseline file; does not gate)",
    )
    ap.add_argument(
        "-v",
        "--verbose",
        action="store_true",
        help="list every occurrence (not just violations)",
    )
    args = ap.parse_args()

    baseline_file = args.baseline_file or str(
        Path(__file__).resolve().parent / "doc_pointer_lint_baseline.txt"
    )

    result = run_census(args.repo_root)

    if args.print_count:
        print(len(result.doc_bare_crate_ratchet))
        return 0

    try:
        baseline = load_baseline(baseline_file)
    except (OSError, ValueError) as exc:
        print(f"doc_pointer_lint.py: {exc}", file=sys.stderr)
        return 2

    ratchet_count = len(result.doc_bare_crate_ratchet)

    print("== scripts/doc-pointer-lint.sh census ==")
    print(f"files scanned                              : {len(discover_files(args.repo_root))}")
    print(f"doc-comment linked crate:: pointers (info)  : {result.doc_linked_crate_count}")
    print(f"doc-comment bare crate:: pointers (total)   : {len(result.doc_bare_crate_all)}")
    print(f"  excluded by opt-out/marker                : {len(result.doc_bare_crate_excluded)}")
    print(f"  ratchet-relevant (role 2)                  : {ratchet_count}")
    print(f"  pinned baseline                            : {baseline}")
    print(f"plain-comment bare crate:: pointers (info)  : {result.plain_bare_crate_count}")
    print(f"plain-comment bracket-link violations (role 1): {len(result.plain_bracket_violations)}")
    print()

    if args.verbose and result.doc_bare_crate_excluded:
        print("doc-comment bare crate:: pointers excluded from the ratchet:")
        for occ, reason in result.doc_bare_crate_excluded:
            print(f"  {occ.path}:{occ.line}  [{reason}]  `{occ.span}`")
        print()

    ok = True

    print("-- role 1 (vyse): plain `//` comment bracket-link, must be 0 --")
    if result.plain_bracket_violations:
        ok = False
        print(f"FAIL: {len(result.plain_bracket_violations)} occurrence(s):")
        for occ in result.plain_bracket_violations:
            print(f"  {occ.path}:{occ.line}  `{occ.span}`")
    else:
        print("PASS: 0 occurrences.")
    print()

    print("-- role 2 (luxp): doc-comment bare crate:: pointers, ratchet vs baseline --")
    if ratchet_count > baseline:
        ok = False
        print(f"FAIL: {ratchet_count} > baseline {baseline} (+{ratchet_count - baseline}).")
        print("New (or newly-unmarked) occurrences:")
        if args.verbose:
            for occ in result.doc_bare_crate_ratchet:
                print(f"  {occ.path}:{occ.line}  `{occ.span}`")
        else:
            print("  (re-run with -v to list every ratchet-relevant occurrence)")
        print()
        print(
            "Fix: convert to an intra-doc link (AGENTS.md's `crate::…` "
            "pointer 節), or if this span is a confirmed opt-out (removed "
            "marker / tests:: target / rustdoc-blind position verified by "
            "the わざと壊して確かめる procedure), mark it with "
            "`doc-pointer-lint:ignore: <reason>` on the same line."
        )
    else:
        print(f"PASS: {ratchet_count} <= baseline {baseline}.")
        if args.verbose:
            for occ in result.doc_bare_crate_ratchet:
                print(f"  {occ.path}:{occ.line}  `{occ.span}`")
    print()

    if ok:
        print("PASS: both roles satisfied.")
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
