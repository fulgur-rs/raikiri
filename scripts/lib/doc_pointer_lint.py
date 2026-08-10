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
     rewrite the line as a bare code span (or, for a non-link bracket use
     that isn't a `crate::…` pointer at all, wrap it in backticks — Step C
     of `analyze_line()` strips backtick spans before the bracket step
     ever runs, so a genuinely non-link `[...]` written as `` `[...]` ``
     is never flagged). `_BARE_BRACKET_RE` matches ANY bracket content
     (not just identifier/path-shaped text — see its own comment for why:
     an earlier, narrower version of this pattern silently missed
     `[two words]` / `[foo-bar]`-shaped brackets, a §8.3 Codex final
     review finding), with two narrow exclusions: `#[attr]` / `&[T]`
     (Rust attribute/slice syntax, not a markdown-link look-alike) and
     `[text](url)` (a genuine Markdown inline link to an explicit URL —
     not currently present in `crates/*/src` plain comments, checked at
     authoring time, but excluded defensively). Block comments (`/* */`)
     — which this script does not scan at all — were independently
     confirmed to hold 0 `crate::` references (bd raikiri-spike-vyse's
     original census); if that construct is ever introduced, re-check
     before trusting role 1's output blindly.

     Two further known scan-scope gaps (bd raikiri-spike-1ghc, from bd
     raikiri-spike-luxp §8.2 roborev-refine iter3's independent finding),
     both re-verified inert (0, or 1 harmless, live hits) and left as a
     documented caveat rather than a logic change. They pull in opposite
     directions — gap 1 is under-scanning (a hazard could go uncaught),
     gap 2 is over-flagging (legitimate Rust syntax could be misreported
     as a role-1 violation) — but neither currently changes this script's
     output on this repo:

       - **Line-position gap**: `classify_line()` classifies on
         `raw.lstrip()`, so a line with code *before* a trailing `//`
         comment (e.g. `let x = 1; // see [Foo]`) is classified `code`
         outright and the trailing comment is never scanned by either
         role. This is a different axis (line position) from the `/* */`
         exclusion above (comment syntax), even though both are the "same
         hazard" by AGENTS.md's "never write this pattern" framing.
         Repo-wide at filing/re-verification time: exactly one shape
         match, `crates/raikiri-paint/src/text.rs`'s
         `// &[i16] (anyrender::NormalizedCoord alias)`, harmless because
         `[i16]` immediately follows `&`, so it would already be excluded
         by the `(?<![#&])` pattern above even if this line were scanned.
       - **Exclusion-prefix gap**: `(?<![#&])` above only excludes a `[`
         immediately preceded by `#` or `&`; `&mut [T]` or
         `impl AsRef<[u8]>` (a token sits between the `&`/generic bracket
         and the `[`) are not excluded by the pattern as written — such a
         shape in a `plain` `//` comment would be misreported as a role-1
         violation (a false FAIL on ordinary Rust syntax, not a missed
         hazard). Repo-wide at filing/re-verification time: 0 hits.

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
# here): a bare bracket-link. AGENTS.md's rule for plain `//` comments is
# blanket — "bracket link 構文 (`[...]` / `` [`...`] ``) を一切書かない" —
# not conditioned on the bracket content looking like a Rust path/ident.
# §8.3 Codex final review (bd raikiri-spike gate iter): an earlier version
# of this pattern only matched identifier/path-shaped content
# (`[A-Za-z_][A-Za-z0-9_:<>]*`), which silently let `[two words]` and
# `[foo-bar]` — anything with a space, hyphen, or other punctuation —
# through role 1 undetected, contradicting the "一切" (unconditionally)
# in the rule it's meant to enforce. Matches ANY non-bracket, non-newline
# content between `[` and `]` now, so shape is no longer a loophole.
#
# Two exclusions are preserved from before (content-shape broadening does
# not touch these — see the walkthrough in this module's test suite):
#   - `(?<![#&])`: a `[` immediately preceded by `#` (Rust attribute,
#     `#[cfg(test)]`) or `&` (slice type written without backticks,
#     `&[T]`) is not a markdown/intra-doc-link-shaped bracket at all, and
#     mattered even more once content-shape stopped doing that filtering
#     for us — `#[derive(Debug, Clone)]`'s content ("derive(Debug,
#     Clone)") would otherwise now match too.
#   - `(?!\()`: a `[text](url)` immediately followed by `(` is a genuine
#     Markdown inline link to an explicit URL, a different (and
#     legitimate) construct from an intra-doc-link look-alike — not
#     currently present in `crates/*/src` plain comments (checked at
#     authoring time, see this module's earlier docstring note), but
#     excluded defensively now that content-shape alone can no longer
#     rule it out either.
# A bracket containing only digits (a footnote-style `[1]`) is
# deliberately NOT exempted — content-shape exemptions are exactly what
# let the space/hyphen gap through in the first place; the remedy for a
# legitimate non-link bracket use in a plain comment is the same as for
# any other: wrap it in backticks (`` `[1]` ``), which Step C already
# strips before this step ever runs.
_BARE_BRACKET_RE = re.compile(r"(?<![#&])\[([^\[\]\n]+)\](?!\()")

# Anchored to an actual `//`-introduced marker (mirrors
# scripts/lib/patch_coverage.py's comment_part()-anchored COV_IGNORE_RE),
# not a bare substring search — every line this runs against is already a
# `doc`/`plain` comment line in its own right, so the marker is written as
# a second, nested `//` fragment on that line (e.g. `` /// `crate::foo::Bar`
# // doc-pointer-lint:ignore: reason ``), the same way `cov:ignore:` reads
# as a trailing comment on a code line. Without this anchor, the marker
# text appearing anywhere on the line (e.g. inside prose *describing* the
# marker itself) would falsely exempt an unrelated occurrence.
_IGNORE_MARKER_RE = re.compile(r"//\s*doc-pointer-lint:ignore:\s*(\S.*)$")


@dataclass
class LineFindings:
    linked_spans: list[str] = field(default_factory=list)  # from [`...`] or bare [Ident]
    bare_backtick_spans: list[str] = field(default_factory=list)  # remaining `...`


def analyze_line(raw: str, *, in_backtick: bool = False) -> tuple[LineFindings, bool]:
    """Extract link-shaped and bare-code-span occurrences from one line.

    `in_backtick`: True if a backtick code span opened on a *previous*
    line of the same contiguous comment run and has not yet closed —
    i.e. this line's leading text, up to its first backtick, is itself
    still inside that span and must not be scanned for brackets at all.
    Returns `(findings, still_in_backtick)`, where `still_in_backtick` is
    True iff a span opened on (or carried into) this line remains
    unclosed at end-of-line, for the caller to pass to the next line.

    This carry-over matters because this codebase's dense prose comments
    routinely word-wrap a single backtick-quoted CSS grammar excerpt
    across several `//`/`///` lines, e.g. (crates/raikiri-style/src/
    property.rs, condensed):

    ```
    // grammar: `auto | <length-percentage [0,∞]> | min-content | max-content
    // | fit-content(<length-percentage>)` のうち ...
    ```

    Without carry-over, the first line above looks — in isolation — like
    it has one *unpaired* backtick, so nothing on it gets recognized as
    "inside a code span"; `[0,∞]` would then wrongly reach the bare-bracket
    step and get flagged as a role-1 violation, even though it is (once
    the two lines are read as the single quoted phrase they are) already
    safely inside backticks. bd raikiri-spike gate §8.3 Codex final review
    surfaced this the moment role 1's bracket matching was broadened to
    catch non-identifier-shaped content (this exact `[0,∞]` shape was one
    of the false negatives that broadening fixed) — a version of this
    function that both scanned in isolation *and* had already stopped
    requiring identifier-shaped content would have then falsely flagged,
    and an earlier pass of the accompanying fix did, in fact, mechanically
    "fix" it by adding a second, nested, syntactically-broken layer of
    backticks. This function exists specifically so that doesn't happen.

    Order otherwise matters the same way it always has: each step
    consumes (blanks out) what it matched before the next step runs, so a
    backtick span already claimed by the bracket-wrapping-backtick form
    isn't re-counted as a bare backtick span, and neither of those can
    leave stray brackets for the bare-bracket step to misfire on (e.g.
    `` `&[Declaration]` `` is fully consumed by step C before step E ever
    sees it).
    """
    findings = LineFindings()
    work = raw

    if in_backtick:
        closer = work.find("`")
        if closer == -1:
            # The whole line is still inside the carried-over span —
            # nothing on it is eligible for any of the steps below.
            return findings, True
        # Blank out through (and including) the closing backtick so the
        # steps below never see it as if it were a fresh opener.
        work = " " * (closer + 1) + work[closer + 1 :]

    def _consume_linked(m: re.Match[str]) -> str:
        findings.linked_spans.append(m.group(1))
        return " " * len(m.group(0))

    work = _BRACKET_BACKTICK_RE.sub(_consume_linked, work)

    def _consume_backtick(m: re.Match[str]) -> str:
        findings.bare_backtick_spans.append(m.group(1))
        return " " * len(m.group(0))

    work = _BACKTICK_RE.sub(_consume_backtick, work)

    # `_BACKTICK_RE.sub` above pairs backticks left-to-right and blanks
    # each complete pair, so at most one backtick can be left in `work`
    # afterwards (a run of 3, 5, 7... backticks pairs down to exactly 1
    # leftover; an even run pairs down to 0). One leftover means a new
    # span opened on this line and didn't close before end-of-line —
    # everything from that backtick to end-of-line is *inside* that span
    # (it's the text between the opener and wherever the closer ends up
    # landing on a later line), so it must be blanked out here too, the
    # same way the carried-in prefix was blanked above, before the
    # bare-bracket step runs. Without this, a bracket following a
    # same-line span-opener (e.g. `` `<length [0,∞]> | `` — the span
    # opens at `` `<length `` and doesn't close until "...thick`" on the
    # *next* line) would still reach the bare-bracket step unprotected;
    # only content *preceding* an unclosed opener on this line, and
    # content on a *later* line once the carried-in prefix above is
    # blanked, was covered without this step.
    still_in_backtick = False
    opener = work.rfind("`")
    if opener != -1 and work.count("`") == 1:
        still_in_backtick = True
        work = work[:opener] + " " * (len(work) - opener)

    for m in _BARE_BRACKET_RE.finditer(work):
        findings.linked_spans.append(m.group(1))

    return findings, still_in_backtick


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
    # Carried across consecutive lines of the *same* comment kind (see
    # analyze_line()'s docstring for why: a backtick span word-wrapped
    # across several `//`/`///` lines must not have its interior treated
    # as bracket-scannable just because the census is line-based). Reset
    # on any `code` line and on a `doc`<->`plain` kind change — a comment
    # run authored as one continuous phrase is never observed to switch
    # comment style mid-span in this codebase, and a code line ends any
    # comment run outright.
    in_backtick = False
    prev_kind: str | None = None
    for lineno, raw in enumerate(text.splitlines(), start=1):
        lstripped = raw.lstrip()
        kind = classify_line(lstripped)
        if kind == "code":
            in_backtick = False
            prev_kind = None
            continue
        if kind != prev_kind:
            in_backtick = False
        findings, in_backtick = analyze_line(raw, in_backtick=in_backtick)
        prev_kind = kind
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


def run_census(repo_root: str, files: list[str] | None = None) -> CensusResult:
    """Census `files` (repo-relative paths), or `discover_files(repo_root)`
    if `files` is omitted. Accepting a pre-computed file list lets a caller
    that also wants the file *count* (main(), for the summary line) reuse a
    single `discover_files()` call instead of running the glob twice."""
    if files is None:
        files = discover_files(repo_root)
    results = []
    for rel_path in files:
        text = Path(repo_root, rel_path).read_text(encoding="utf-8")
        results.append(census_file(rel_path, text))
    return merge(results)


def load_baseline(baseline_file: str) -> int:
    """Read the pinned ratchet baseline: the first non-blank, non-`#`-comment
    line, parsed as an int. Raises `FileNotFoundError` (an `OSError`
    subclass) if `baseline_file` doesn't exist, and `ValueError` if it
    exists but contains no such line (or that line isn't a valid int) —
    both are caught by main() and reported as a tooling error (exit 2),
    not a gate failure."""
    for line in Path(baseline_file).read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        return int(line)
    raise ValueError(f"{baseline_file}: no non-comment integer line found")


def evaluate_gate(result: CensusResult, baseline: int) -> tuple[bool, bool]:
    """The gate's PASS/FAIL decision, isolated from argument parsing and
    printing so it's directly unit-testable. Returns `(role1_ok, role2_ok)`:

      - role1_ok: True iff there are 0 plain-comment bracket-link
        violations (vyse's hard-zero rule — no baseline involved).
      - role2_ok: True iff the ratchet-relevant doc-comment bare `crate::…`
        pointer count is at or below `baseline` (luxp's ratchet).

    `main()`'s overall exit code is 0 iff both are True, else 1 — see its
    body for the exact mapping (this function does not decide exit codes
    itself, to keep it free of any print/argparse/sys dependency)."""
    role1_ok = not result.plain_bracket_violations
    role2_ok = len(result.doc_bare_crate_ratchet) <= baseline
    return role1_ok, role2_ok


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

    files = discover_files(args.repo_root)
    result = run_census(args.repo_root, files=files)

    if args.print_count:
        print(len(result.doc_bare_crate_ratchet))
        return 0

    try:
        baseline = load_baseline(baseline_file)
    except (OSError, ValueError) as exc:
        print(f"doc_pointer_lint.py: {exc}", file=sys.stderr)
        return 2

    ratchet_count = len(result.doc_bare_crate_ratchet)

    summary_rows = [
        ("files scanned", len(files)),
        ("doc-comment linked crate:: pointers (info)", result.doc_linked_crate_count),
        ("doc-comment bare crate:: pointers (total)", len(result.doc_bare_crate_all)),
        ("  excluded by opt-out/marker", len(result.doc_bare_crate_excluded)),
        ("  ratchet-relevant (role 2)", ratchet_count),
        ("  pinned baseline", baseline),
        ("plain-comment bare crate:: pointers (info)", result.plain_bare_crate_count),
        ("plain-comment bracket-link violations (role 1)", len(result.plain_bracket_violations)),
    ]
    label_width = max(len(label) for label, _ in summary_rows)

    print("== scripts/doc-pointer-lint.sh census ==")
    for label, value in summary_rows:
        print(f"{label.ljust(label_width)} : {value}")
    print()

    if args.verbose and result.doc_bare_crate_excluded:
        print("doc-comment bare crate:: pointers excluded from the ratchet:")
        for occ, reason in result.doc_bare_crate_excluded:
            print(f"  {occ.path}:{occ.line}  [{reason}]  `{occ.span}`")
        print()

    role1_ok, role2_ok = evaluate_gate(result, baseline)

    print("-- role 1 (vyse): plain `//` comment bracket-link, must be 0 --")
    if not role1_ok:
        print(f"FAIL: {len(result.plain_bracket_violations)} occurrence(s):")
        for occ in result.plain_bracket_violations:
            print(f"  {occ.path}:{occ.line}  `{occ.span}`")
    else:
        print("PASS: 0 occurrences.")
    print()

    print("-- role 2 (luxp): doc-comment bare crate:: pointers, ratchet vs baseline --")
    if not role2_ok:
        print(f"FAIL: {ratchet_count} > baseline {baseline} (+{ratchet_count - baseline}).")
        # NOT a new-vs-baseline diff: the baseline is a plain integer, not a
        # line set, so this script has no record of *which* occurrences it
        # was measured against — it structurally cannot tell "new since the
        # baseline was pinned" apart from "was already there but happens to
        # push the total over." This lists every current ratchet-relevant
        # occurrence; per scripts/lib/doc_pointer_lint_baseline.txt's own
        # guidance (a foreign merge can move this count without this
        # branch's own commits touching a single one of these lines),
        # isolate what actually changed by diffing this same command's
        # output against a run at the baseline-pinning commit.
        print(
            "All ratchet-relevant occurrences (not a new-vs-baseline diff — "
            "compare against a run at the baseline-pinning commit to "
            "isolate what changed):"
        )
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

    if role1_ok and role2_ok:
        print("PASS: both roles satisfied.")
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
