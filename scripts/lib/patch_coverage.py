#!/usr/bin/env python3
"""scripts/lib/patch_coverage.py — patch-coverage line classifier.

Called by `scripts/patch-coverage.sh` after it has already produced an lcov
report. This module owns the two pieces of logic that don't belong in shell:

  1. Parsing a `-U0` unified diff to get the exact set of *added* line
     numbers per file (gate.md §8.1.1's "merge-base diff extraction").
  2. Deciding whether an uncovered added line is exempted by a
     `// cov:ignore: <reason>` annotation (the "exemption mechanism").

# The `cov:ignore` scoping rule this script implements

A `cov:ignore:` marker must be the start of an actual `//` comment (found by
`comment_part()`, which tracks string/char literals the same way
`code_only()` does, so a string *containing* the text `cov:ignore:` doesn't
count) and must have a non-empty reason after the colon — `// cov:ignore:`
with nothing following it does not match. It is either:

  - a trailing comment on a code line (`foo(); // cov:ignore: reason`) —
    exempts that line only, or
  - a comment-only line (optionally part of a multi-line comment run) —
    exempts the block of code that *immediately follows* the comment run.
    "Block" is computed by tracking `{`/`(`/`[` vs `}`/`)`/`]` depth
    (string/char literal contents and `//` line comments are stripped
    first, so brace characters inside a `"..."` don't confuse the count;
    this stripping carries open-string-literal state across physical
    lines, so a backslash-continued multi-line string literal doesn't
    leak its prose's punctuation into the depth count either — bd
    the earlier change):
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
for (`lens-matrix.md` §fix-application model: "adding cov:ignore: quality
lens only"). New annotations should still be written as tightly as
possible (immediately before the minimal exempted line/arm) — this is a
known limitation, not a license to annotate loosely.

Lines that never appear in the lcov DA table at all (blank lines, pure
comments) are trivially exempt: absence for a normal source line means "not
an executable line, no coverage obligation". Absence for an *entire file*
(zero `SF:` records) is split into two cases, verified empirically against
this workspace's `cargo-llvm-cov 0.8.7` output rather than assumed:

  - A file cargo-llvm-cov's target set didn't build/run under a plain
    `cargo llvm-cov --workspace` (mirroring `cargo test --workspace`) at
    all. Two source-level shapes land here, handled identically by
    `main()` (they share one code path — see the `else` arm of the
    `da_map is None` branch): `benches/*.rs` (a changed bench file has
    *zero* chance of being exercised by this coverage run — "never
    instrumented" really does mean "never ran under any gate check"), and
    an ordinary `src/*.rs` file that happens to have zero executable
    lines at all — a "pure declaration" file: only `use` statements,
    struct/enum definitions, and doc comments, e.g.
    `crates/raikiri-html/src/types.rs` (there was simply never anything
    for cargo-llvm-cov to instrument, not "ran but wasn't measured").
    Every added *code* line is reported **uncovered** for the bench case
    (intentional, not a bug: see the earlier change's gate history for
    why a bench-file diff hitting this is expected and needs either a
    `--benches`-style follow-up or a per-diff escalation) — and vacuously
    never triggered for a genuine pure-declaration file, since by
    definition it has no code lines to report. Blank/comment added lines
    are the one exception in *both* sub-shapes: see
    `classify_no_lcov_record_lines()` below for why those can never be
    "uncovered" even with no `DA:` table to check them against — this is
    what fixes the false positive a pure-declaration file's doc-comment-only
    diff used to produce.
  - Cargo `test`/`example` **targets** (auto-discovered `tests/*.rs` /
    `examples/*.rs` integration-test and example binaries): confirmed by
    direct measurement (`cargo llvm-cov -p raikiri --lcov`, then
    `cargo llvm-cov report --summary-only`) that integration-test-target
    source files — as opposed to a `#[cfg(test)] mod tests` block *inside*
    a `src/*.rs` file, which reports normally because it shares that
    file's SF record — **never get their own `SF:` record**, regardless
    of whether their tests ran and passed. `crates/raikiri/tests/
    external_consumer.rs` and `crates/raikiri/tests/parse_html_limits.rs`
    both ran (visible in the `cargo test` output) yet neither appears in
    the lcov export at all; the library functions they call *do* show up,
    correctly attributed to their own `src/*.rs` location. This is the
    observed *behavior*, not a diagnosed mechanism — this script does not
    know cargo-llvm-cov's internal reasoning for excluding these files
    from the report, only that it consistently does, on this
    cargo-llvm-cov version, for both files checked. Treat it as an
    empirical fact to route around, not an intentional-design claim to
    cite further. Added lines in such a file are reported as
    **unreported** (a distinct, non-failing category) rather than
    uncovered — flagging every new integration test as a coverage
    violation would be a standing false positive, not a signal.

    Which files count as a `test`/`example` target is decided by
    cross-referencing `cargo metadata`'s own target list (see
    `structurally_unreported_paths()`), not by a `tests/`/`examples/`
    path-shape guess: a path merely *containing* a directory component
    named `tests` (e.g. an ordinary `src/tests/fixtures.rs` module,
    `mod tests;` declared from its parent) is not a Cargo target and must
    not be exempted — that would mask a real coverage gap in production
    code. the earlier fix addressed this false-negative risk in
    the prior path-regex-only implementation.
  - `tests.rs` / `*_tests.rs` / `*-tests.rs` **files** anywhere, e.g. a
    `#[cfg(test)] mod tests;` module moved out of its parent into
    `<parent>/tests.rs`: these match the filename half of cargo-llvm-cov's
    built-in default `--ignore-filename-regex`, so they get no SF: record
    even though their tests ran. Also classified **unreported**; see
    `is_structurally_unreported()`.

Git's moved-block colors are only candidates for a move exemption. A
candidate must belong to a complete unchanged function deleted elsewhere
in this diff, with the same Cargo target, logical module, attributes, and
name bindings. Inline modules may become files while preserving this
execution context. Matching expressions or partial function bodies cannot
establish preserved execution context. See
`collect_moved_added_lines()`.
"""

from __future__ import annotations

import argparse
import json
import posixpath
import re
import subprocess
import sys
from collections import Counter
from pathlib import Path, PurePosixPath
from dataclasses import dataclass, field

COMMENT_LINE_RE = re.compile(r"^\s*//")
# A single-line `mod` / `use` declaration: never executable code.
MODULE_DECL_LINE_RE = re.compile(r"^\s*(pub(\([^)]*\))?\s+)?(use\s[^;]*|mod\s+\w+\s*);\s*$")
ATTRIBUTE_LINE_RE = re.compile(r"^\s*#!?\[.*\]\s*$")
# Anchored to the start of an actual `//` comment (see comment_part() below,
# which finds that `//` while honoring string/char literals) and requires a
# non-empty reason after the colon. A bare substring match (the previous
# form of this regex) would (a) fire on a string literal that merely
# *contains* the text `cov:ignore:`, since it never checked the match was
# inside a real comment, and (b) accept `// cov:ignore:` with nothing after
# the colon as a valid exemption. review finding.
COV_IGNORE_RE = re.compile(r"^//\s*cov:ignore:\s*(\S.*)$")
# Filename half of cargo-llvm-cov 0.8.7's built-in default
# `--ignore-filename-regex` (read from the installed binary; opt-out is
# `--no-default-ignore-filename-regex`). Files matching it get no SF: record.
LLVM_COV_IGNORED_FILENAME_RE = re.compile(r"(^|/)(tests\.rs|[0-9a-zA-Z_-]+[_-]tests\.rs)$")
HUNK_HEADER_RE = re.compile(r"^@@ -\d+(?:,\d+)? \+(\d+)(?:,(\d+))? @@")


def _char_literal_end(line: str, i: int) -> int | None:
    """If `line[i]` is a `'` that opens a genuine Rust char literal (shape
    `'x'`: one character, or one of Rust's fixed-shape backslash escapes,
    closed by another `'`), return the index of that closing `'`.
    Otherwise return None — the `'` is a lifetime sigil (`'i`, `'t`,
    `'static`, `'a_long_name`, ...), not a literal, and callers must not
    treat it as opening a string-like span.

    Bounded lookahead only (never more than a handful of characters),
    rather than scanning the rest of the line for a matching quote: a char
    literal's body is always short — one character, or one of `\\n` `\\t`
    `\\r` `\\\\` `\\'` `\\"` `\\0`, `\\xHH`, or `\\u{...}` — while a
    lifetime name can be arbitrarily long and is never itself followed by
    a closing `'`. Bounding the lookahead is what keeps a real lifetime
    (however long) from ever being mistaken for an unterminated char
    literal that swallows the rest of the line — the exact failure mode
    this function replaces (see module docstring / code_only()).
    """
    n = len(line)
    j = i + 1
    if j >= n or line[j] == "'":
        return None  # nothing after `'`, or `''` — not a char literal
    if line[j] == "\\":
        k = j + 1
        if k >= n:
            return None
        if line[k] == "x":
            k += 3  # \xHH: 'x' + 2 hex digits (not validated as hex)
        elif line[k] == "u":
            if k + 1 >= n or line[k + 1] != "{":
                return None
            close_brace = line.find("}", k + 1)
            if close_brace == -1:
                return None
            k = close_brace + 1
        else:
            k += 1  # single-char escape: \n \t \r \\ \' \" \0 ...
    else:
        k = j + 1  # ordinary single character
    return k if k < n and line[k] == "'" else None


def comment_part(line: str) -> str | None:
    """Return the `//`-comment suffix of `line` (starting at the `//`), or
    None if the line has no real (not-inside-a-string-or-char-literal) `//`.

    Shares the same string/char-literal tracking as code_only() below —
    kept as a separate function (rather than deriving one from the other)
    because callers want different things: code_only() wants everything
    *except* the comment, this wants *only* the comment.

    Best-effort, same caveat as code_only(): does not handle raw strings
    (`r"..."`, `r#"..."#`) specially. A `"` inside a raw string's body can
    be mistaken for a literal's closing quote, after which a `//` still
    physically inside that raw string could be treated as a real comment
    start. Practically this means a `cov:ignore:`-shaped sequence would
    have to appear *inside* the text of a raw string for a false exemption
    to result — considered acceptable risk for this repo's actual content
    (no raw string in this codebase contains that exact substring, per a
    repo-wide grep at review time) rather than implementing a full Rust
    raw-string lexer here. review finding (the earlier change
    consolidated gate update); tracked as a known limitation rather
    than fixed in this pass.
    """
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
        if c == "'":
            end = _char_literal_end(line, i)
            if end is None:
                # Lifetime sigil, not a char literal — leave it as
                # ordinary code and keep scanning normally instead of
                # entering "inside a literal" state.
                i += 1
                continue
            i = end + 1
            continue
        if c == '"':
            in_str = True
            quote = c
            i += 1
            continue
        if c == "/" and i + 1 < n and line[i + 1] == "/":
            return line[i:]
        i += 1
    return None


def cov_ignore_reason(line: str) -> str | None:
    """Return the reason text if `line` carries a valid `cov:ignore:` marker
    in an actual `//` comment with a non-empty reason, else None."""
    part = comment_part(line)
    if part is None:
        return None
    m = COV_IGNORE_RE.match(part)
    return m.group(1) if m else None


def code_only(line: str, in_str: bool = False, quote: str = "") -> tuple[str, bool, str]:
    """Strip string/char literal contents and a trailing `//` comment.

    `in_str`/`quote` are the string-literal state the caller already has
    open coming into `line` (e.g. a Rust string literal — commonly a
    backslash-continued `assert!`/`panic!` message — that didn't close on
    a prior physical line); the returned `(code, in_str, quote)` tuple's
    last two elements are that same state as it stands at the end of
    `line`, for the caller to pass into the next physical line. A caller
    that only has one line to look at (no cross-line context) can ignore
    the returned state and rely on the `False`/`""` defaults, which behave
    exactly like a per-line-only parse.

    This state must be threaded across physical lines by any caller doing
    multi-line brace-depth bookkeeping (see `compute_exempt_lines()`
    below): a Rust string literal can itself contain an unescaped `)`,
    `]`, or `}` in its text (most commonly prose in a multi-line assert/
    panic message), and if the caller re-parses each line from a fresh
    "not in a string" state, that punctuation is miscounted as real code
    syntax. Resetting per line was exactly this function's bug before
    `in_str`/`quote` became threadable (the earlier change).

    Best-effort: does not handle raw strings (`r"..."`) or byte-string
    prefixes specially, which can misparse in rare cases. Good enough for
    brace-depth bookkeeping, not a real Rust lexer.

    A `'` is only treated as opening a char literal if `_char_literal_end()`
    finds a matching closing `'` within its bounded lookahead; otherwise
    it's a lifetime sigil (`'i`, `'t`, `'static`, ...) and is passed through
    as ordinary code instead of putting the scanner into "inside a
    literal" state. Rust generic signatures like `Parser<'i, 't>` or
    `ParseError<'i, Self::Error>` are full of these bare, unpaired
    apostrophes; treating one as an unterminated char literal used to
    silently drop every character after it on the line (and, since this
    function feeds brace_delta() for `cov:ignore:` block-scope tracking,
    could truncate an exempted block early — see counter_style.rs's
    `QualifiedRuleParser::parse_block` for a real signature that triggered
    this). Because a lone `'` is always resolved on its own line this way
    (either consumed as a genuine char literal, closing before the line
    ends, or passed through as ordinary code), `in_str`/`quote` state is
    only ever left open across a line boundary for a `"` (double-quoted
    string) literal in practice — a Rust char literal can never
    legitimately span a physical line, so there is no equivalent
    cross-line case for `'` to thread. The trailing `if quote != '"':`
    reset below is a defensive backstop for that invariant rather than a
    load-bearing branch under normal input.

    Threading `"` state across lines like this means an existing
    raw-string misparse (an odd number of literal `"` inside an
    `r#"..."#` body being mistaken for a literal boundary) can now also
    propagate across a line boundary instead of self-correcting every
    line — accepted for the same reason the module docstring already
    accepts other over-exemption risk in this heuristic: the failure
    direction is over-exemption, and unlike a lifetime apostrophe, a
    legitimate double-quoted string *does* routinely continue across
    lines (a backslash-continued `assert!`/`panic!` message, most
    commonly), so there's no equivalent "never legitimate" rule available
    to suppress it the way there is for `'`.
    """
    out = []
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
                quote = ""
            i += 1
            continue
        if c == "'":
            end = _char_literal_end(line, i)
            if end is None:
                # Lifetime sigil — keep as ordinary code.
                out.append(c)
                i += 1
                continue
            i = end + 1
            continue
        if c == '"':
            in_str = True
            quote = c
            i += 1
            continue
        if c == "/" and i + 1 < n and line[i + 1] == "/":
            break
        out.append(c)
        i += 1
    if quote != '"':
        in_str = False
        quote = ""
    return "".join(out), in_str, quote


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
            if cov_ignore_reason(raw) is not None:
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
            # in_str/quote thread code_only()'s string-literal state across
            # physical lines of this block (starting fresh at the block's
            # first line): a backslash-continued Rust string literal that's
            # still open at the end of line j must not have its state reset
            # before line j+1 is parsed, or punctuation in the continuation
            # line's prose gets miscounted as real brace/paren/bracket
            # syntax and truncates the block early (the earlier change).
            in_str = False
            quote = ""
            while True:
                code, in_str, quote = code_only(lines[j], in_str, quote)
                delta = brace_delta(code)
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
        if not COMMENT_LINE_RE.match(raw) and cov_ignore_reason(raw) is not None:
            exempt.add(idx + 1)

    return exempt


def classify_no_lcov_record_lines(
    added_lines: list[int],
    file_lines: list[str],
    exempt: set[int],
) -> tuple[list[int], list[int]]:
    """Classify added lines for a file with zero lcov records at all (no
    `SF:` block — `da_map is None` in `main()`) that is *not* a
    structurally-unreported Cargo test/example target. Returns
    `(uncovered, exempted)`.

    When there *is* a `DA:` table to consult (the normal branch in
    `main()`), a blank, pure-comment, or Rust attribute added line is
    filtered out before the `exempt` check even runs, simply because such
    a line never has a `DA:` record in the first place (`if ln not in
    da_map: continue`) — it was never "executable", so it was never
    eligible to be "uncovered". With no `DA:` table at all, that filter
    can't be expressed the same way; this function reconstructs the same
    outcome structurally, using `COMMENT_LINE_RE`, `ATTRIBUTE_LINE_RE`,
    and a blank-line check. A line skipped this way is reported in neither
    list — deliberate parity with the normal branch, where such a line
    doesn't appear in `uncovered`/`exempted` either (it still counts toward
    the file's `added_lines` total, same as there).

    `// cov:ignore:` still exempts a *code* line exactly as before; this
    function doesn't touch that path. It only stops comments, blank lines,
    attributes, and module declarations from being misclassified as
    uncovered in a file that had no executable-line table to filter them
    out with.

    Known limitation, not fixed here: a non-comment, non-blank *added*
    line that also happens to be non-executable in real Rust (e.g. a
    `pub bar: u32,` struct field, a bare `}` closing a block) is still
    classified `uncovered` unless `cov:ignore:`-annotated. Distinguishing
    "syntactically code-shaped" from "actually executable" in general
    requires a real Rust parser, not a regex; the structural checks here
    cover only comments, blanks, attributes, and module declarations.
    Files that mix zero-SF-record status with genuinely new field/variant
    declarations will still need a `// cov:ignore:` or an escalation for
    those specific lines, same as before this fix.
    """
    uncovered: list[int] = []
    exempted: list[int] = []
    for ln in added_lines:
        raw = file_lines[ln - 1] if 0 <= ln - 1 < len(file_lines) else ""
        if (
            not raw.strip()
            or COMMENT_LINE_RE.match(raw)
            or MODULE_DECL_LINE_RE.match(raw)
            or ATTRIBUTE_LINE_RE.match(raw)
        ):
            continue
        if ln in exempt:
            exempted.append(ln)
        else:
            uncovered.append(ln)
    return uncovered, exempted


class CargoMetadataError(RuntimeError):
    """`cargo metadata` could not be run or its output could not be parsed.

    Raised instead of letting a bare `subprocess`/`json` exception escape:
    this is a gate script whose stderr is read by another agent, so the
    failure needs to name the command and the underlying cause rather than
    surface as a raw traceback.
    """


def load_cargo_metadata(repo_root: str) -> dict:
    """Run `cargo metadata` once for the whole workspace and return the
    parsed JSON.

    Run with `cwd=repo_root` rather than an explicit `--manifest-path
    <repo_root>/Cargo.toml`: `--repo-root` (from `patch-coverage.sh`) is
    `git rev-parse --show-toplevel`, the *git* root, which today also
    happens to hold the workspace manifest but isn't guaranteed to by
    anything checked here. `cargo metadata` itself walks up from `cwd` to
    find the enclosing workspace root, so `cwd=repo_root` gets the right
    answer even if a manifest ever moved, instead of hardcoding an
    assumption that would fail with a confusing "no such manifest" error.

    `--no-deps` keeps this to workspace members (no need to resolve/print
    metadata for every external dependency). `--locked` matches
    `patch-coverage.sh`'s own `cargo llvm-cov --locked` invocation: a stale
    `Cargo.lock` should fail loudly here exactly as it would there, not get
    silently rewritten by this classification step.
    """
    try:
        proc = subprocess.run(
            ["cargo", "metadata", "--no-deps", "--format-version=1", "--locked"],
            cwd=repo_root,
            capture_output=True,
            text=True,
            check=True,
        )
    except (OSError, subprocess.CalledProcessError) as exc:
        stderr = getattr(exc, "stderr", None) or ""
        raise CargoMetadataError(
            f"`cargo metadata` (cwd={repo_root}) failed: {exc}\n{stderr}"
        ) from exc
    try:
        return json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise CargoMetadataError(
            f"`cargo metadata` (cwd={repo_root}) produced unparseable JSON: {exc}"
        ) from exc


def structurally_unreported_paths(metadata: dict, repo_root: str) -> set[str]:
    """Repo-relative source paths of every Cargo `test`/`example` target's
    *entry* file — the ones cargo-llvm-cov's report never lists an SF:
    record for (see module docstring). Driven by `cargo metadata`'s own
    target kinds rather than a path-shape guess, so an ordinary
    `src/tests/fixtures.rs` *module* (not a target) is never mistaken for
    one: only a file cargo itself built as a `"test"` or `"example"` target
    appears here. `"bench"` is deliberately excluded — absence there means
    "never ran", a real gap (see module docstring), not "never reported".

    Known limitation, unmeasured; uncertain cases are counted by default: this only covers
    a target's *entry* file (the one `cargo metadata` names as
    `src_path`). A shared helper module compiled into a test binary but
    included from it (e.g. a hypothetical `tests/common/mod.rs` pulled in
    via `mod common;`) is not itself a target and so is not in this set —
    if cargo-llvm-cov also omits an SF: record for such a file, it would
    be classified `uncovered` (gating) rather than `unreported`. No file
    of that shape exists in this repo to measure against as of this
    writing; revisit empirically if one is added and trips this gate.

    `repo_root` (git's toplevel, uncanonicalized) and `src_path` (from
    `cargo metadata`, canonicalized) are expected to share a literal
    string prefix, but nothing guarantees it — a symlinked checkout or a
    container bind-mount path could make every `src_path.startswith(prefix)`
    fail simultaneously. If that happens the raw (unstripped) `src_path` is
    kept rather than dropped (the earlier coverage fix 2:
    silent-but-diagnosed, not silent-but-invisible), which can never match
    a diff-relative path — every Cargo test/example target would revert to
    gate-failing `uncovered`, repo-wide, with a confusing "add a covering
    test" message for files that structurally can't be covered. A warning
    is printed to stderr when this happens so the failure is at least
    diagnosable instead of silently reappearing through the cargo-metadata
    coupling this function adds. Not observed in this repo as of this
    writing (verified: no symlinks between the git toplevel and the
    workspace root, prefix matches for every real target).
    """
    prefix = repo_root.rstrip("/") + "/"
    paths: set[str] = set()
    total = 0
    mismatched = 0
    for package in metadata.get("packages", []):
        for target in package.get("targets", []):
            kinds = target.get("kind", [])
            if "test" not in kinds and "example" not in kinds:
                continue
            total += 1
            src_path = target.get("src_path", "")
            if src_path.startswith(prefix):
                paths.add(src_path[len(prefix):])
            else:
                mismatched += 1
                paths.add(src_path)
    if mismatched:
        print(
            f"patch_coverage.py: WARNING: cargo-metadata src_path prefix "
            f"mismatch: {mismatched}/{total} Cargo test/example target "
            f"src_path(s) did not start with repo_root prefix {prefix!r}. "
            "These paths are kept unstripped and will not match any "
            "diff-relative path, so the affected test/example targets will "
            "be misclassified as gating 'uncovered' instead of "
            "'unreported'. See structurally_unreported_paths()'s docstring.",
            file=sys.stderr,
        )
    return paths


def is_structurally_unreported(path: str, unreported_paths: set[str]) -> bool:
    """True for paths cargo-llvm-cov's report never lists an SF: record for,
    independent of whether their tests ran — see the module docstring's
    Cargo `test`/`example` target case. NOT true for `bench` targets,
    where absence instead means "never ran" and should fail.

    Also true for a file whose *name* matches the filename half of
    cargo-llvm-cov's default `--ignore-filename-regex` (`tests.rs`,
    `*_tests.rs`, `*-tests.rs`) — typically a `#[cfg(test)] mod tests;`
    module file. The directory half of that default regex (`tests/`,
    `examples/`, `benches/` components) is deliberately not mirrored here:
    see `structurally_unreported_paths()` for why a `src/tests/` module
    must not be exempted by path shape alone."""
    return path in unreported_paths or bool(LLVM_COV_IGNORED_FILENAME_RE.search(path))


@dataclass
class FileResult:
    path: str
    added_lines: list[int] = field(default_factory=list)
    uncovered: list[int] = field(default_factory=list)
    exempted: list[int] = field(default_factory=list)
    unreported: list[int] = field(default_factory=list)
    moved: list[int] = field(default_factory=list)
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


# `git diff --color-moved` marks moved lines only through color. Every
# "moved, new side" color slot is forced to one SGR sequence that plain added
# lines never get (they are forced to green), so the parser can tell them
# apart regardless of the user's git color config.
MOVED_COLOR = "bold magenta"
MOVED_SGR = "\x1b[1;35m"
ANSI_RE = re.compile(r"\x1b\[[0-9;]*m")


def parse_moved_added_lines(colored_diff: str) -> dict[str, set[int]]:
    """Parse the colored `-U0` diff produced by `collect_moved_added_lines()`
    into {path: {added line # that git classified as moved}}."""
    result: dict[str, set[int]] = {}
    current_path: str | None = None
    next_line: int | None = None
    for raw in colored_diff.splitlines():
        line = ANSI_RE.sub("", raw)
        if line.startswith("+++ "):
            path = line[4:]
            current_path = None if path == "/dev/null" else (
                path.split("/", 1)[1] if path.startswith("b/") else path
            )
            continue
        if line.startswith("@@"):
            m = HUNK_HEADER_RE.match(line)
            next_line = int(m.group(1)) if m and current_path is not None else None
            continue
        if next_line is None or current_path is None:
            continue
        if line.startswith("+") and not line.startswith("+++"):
            if MOVED_SGR in raw:
                result.setdefault(current_path, set()).add(next_line)
            next_line += 1
    return result


def collect_moved_added_lines(repo_root: str, base: str, head: str,
                             metadata: dict | None = None) -> dict[str, set[int]]:
    """Find Git move candidates backed by a complete unchanged function.

    Git can color a single long expression as moved even when its role
    changed from a pattern to a branch result. Require a deleted function
    with identical tokens and enclosing context before exempting a line.
    This retains function/file extraction, including indentation changes,
    while leaving edited functions and copied code subject to coverage.
    Opaque compiler extensions cannot prove unchanged expansion inputs,
    so their callers remain subject to coverage even with identical tokens.
    """
    config = ["-c", "color.diff.new=green"]
    for slot in (
        "newMoved",
        "newMovedAlternative",
        "newMovedDimmed",
        "newMovedAlternativeDimmed",
    ):
        config += ["-c", f"color.diff.{slot}={MOVED_COLOR}"]
    proc = subprocess.run(
        [
            "git",
            "-C",
            repo_root,
            *config,
            "diff",
            "--no-renames",
            "--color=always",
            "--color-moved=blocks",
            "--color-moved-ws=allow-indentation-change",
            "-U0",
            base,
            head,
            "--",
            "*.rs",
        ],
        capture_output=True,
        text=True,
        check=True,
    )
    candidates = parse_moved_added_lines(proc.stdout)
    if not candidates:
        return {}
    manifests = subprocess.run(
        ["git", "-C", repo_root, "diff", "--name-only", "-z", "--no-renames", base, head,
         "--", "*Cargo.toml"], capture_output=True, text=True, check=True,
    ).stdout.split("\0")
    # Compiler extensions can interpret even formatting as input. Git
    # detects changed bytes without text-mode newline normalization or
    # requiring a version-specific TOML parser.
    if any(PurePosixPath(manifest).name == "Cargo.toml" for manifest in manifests):
        return {}
    config_paths = subprocess.run(
        ["git", "-C", repo_root, "diff", "--name-only", base, head, "--", ".cargo/config", ".cargo/config.toml",
         "*/.cargo/config", "*/.cargo/config.toml", "rust-toolchain", "rust-toolchain.toml", "*Cargo.lock"],
        capture_output=True, text=True, check=True,
    ).stdout.splitlines()
    if config_paths:
        return {}
    added = parse_added_lines(ANSI_RE.sub("", proc.stdout))
    # Reversing the diff can align different shared lines. Parse deletions
    # from the very same forward diff that supplied the move candidates.
    deleted = parse_deleted_lines(ANSI_RE.sub("", proc.stdout))
    old_functions = Counter()
    snapshots = {ref: rust_module_snapshot(repo_root, ref, metadata) for ref in (base, head)}
    if (snapshots[base]["build_inputs"] != snapshots[head]["build_inputs"]
            or snapshots[base]["build_data"] != snapshots[head]["build_data"]
            or any(value is None or has_file_include(value) or has_runtime_file_input(value)
                   for snapshot in snapshots.values() for value in snapshot["build_inputs"].values())):
        return {}
    for path, lines in deleted.items():
        snapshot = snapshots[base]
        for unit in (snapshot["sources"].get(path) or ([], [], {}))[0]:
            _, _, substantive_lines, _ = unit
            # Git can align shared statements as context while moving
            # the surrounding function. Count an affected occurrence;
            # its complete tokens must still match on the new side.
            if substantive_lines & set(lines):
                semantic_key = function_move_key(snapshot, path, unit)
                if semantic_key is not None:
                    old_functions[semantic_key] += 1
    new_functions = []
    new_counts = Counter()
    for path, lines in added.items():
        snapshot = snapshots[head]
        for unit in (snapshot["sources"].get(path) or ([], [], {}))[0]:
            _, token_lines, substantive_lines, _ = unit
            if substantive_lines & set(lines):
                key = function_move_key(snapshot, path, unit)
                if key is not None:
                    new_functions.append((path, key, token_lines))
                    new_counts[key] += 1
    result = {}
    for path, key, token_lines in new_functions:
        # If destinations outnumber deleted sources, the move/copy mapping
        # is ambiguous. Exempt none rather than reuse one deleted function.
        if new_counts[key] <= old_functions[key]:
            eligible = token_lines & candidates.get(path, set())
            if eligible:
                result.setdefault(path, set()).update(eligible)
    return result


def parse_deleted_lines(diff: str) -> dict[str, set[int]]:
    """Read old-side line numbers from a forward zero-context diff."""
    result = {}
    path = None
    next_line = None
    for line in diff.splitlines():
        if line.startswith("--- "):
            path = line[4:]
            path = None if path == "/dev/null" else path.removeprefix("a/")
            next_line = None
        elif line.startswith("@@"):
            match = re.match(r"^@@ -(\d+)(?:,\d+)? \+\d+(?:,\d+)? @@", line)
            next_line = int(match.group(1)) if match and path is not None else None
        elif next_line is not None and path is not None:
            if line.startswith("-"):
                result.setdefault(path, set()).add(next_line)
                next_line += 1
            elif line.startswith(" "):
                next_line += 1
    return result


def rust_identifier(token: str) -> str | None:
    return token.removeprefix("r#") if re.fullmatch(r"(?:r#)?[^\W\d]\w*", token) else None


def rust_move_tokens(source: str) -> list[tuple[str, int]] | None:
    """Tokenize enough Rust syntax to compare complete functions safely.

    Keep literal contents, suffixes and doc attributes. Omit whitespace
    and plain comments; retain line numbers. Unterminated literals/comments fail closed. This is not a
    Rust parser: unsupported item shapes simply receive no exemption.
    """
    tokens = []
    raw_start = re.compile(r'(?:br|cr|r)(#*)"')
    word_or_operator = re.compile(
        r"\w+|<<=|>>=|\.\.=|\.\.\.|::|->|=>|&&|\|\||==|!=|<=|>=|"
        r"\+=|-=|\*=|/=|%=|\^=|&=|\|=|<<|>>|\.\."
    )
    raw_identifier = re.compile(r"r#[^\W\d]\w*")
    literal_suffix = re.compile(r"[^\W\d]\w*")
    number = re.compile(r"[0-9][0-9_]*(?:\.(?:[0-9][0-9_]*|(?!(?:[.\w]))))?(?:[eE][+-]?[0-9][0-9_]*)?\w*")
    i = 0
    line = 1
    while i < len(source):
        start = i
        if source[i].isspace():
            line += source[i] == "\n"
            i += 1
            continue
        if source.startswith("//", i):
            end = source.find("\n", i)
            i = len(source) if end < 0 else end
            if source.startswith("//!", start) or (source.startswith("///", start) and not source.startswith("////", start)):
                prefix = ("#", "!") if source.startswith("//!", start) else ("#",)
                tokens.extend((value, line) for value in prefix + ("[", "doc", "=", json.dumps(source[start:i]), "]"))
            continue
        if source.startswith("/*", i):
            depth = 1
            i += 2
            while i < len(source) and depth:
                if source.startswith("/*", i):
                    depth += 1
                    i += 2
                elif source.startswith("*/", i):
                    depth -= 1
                    i += 2
                else:
                    i += 1
            if depth:
                return None
            if source.startswith("/*!", start) or (source.startswith("/**", start)
                                                    and not source.startswith(("/***", "/**/"), start)):
                prefix = ("#", "!") if source.startswith("/*!", start) else ("#",)
                tokens.extend((value, line) for value in prefix + ("[", "doc", "=", json.dumps(source[start:i]), "]"))
            line += source[start:i].count("\n")
            continue
        literal = False
        raw = raw_start.match(source, i)
        if raw:
            literal = True
            terminator = '"' + raw.group(1)
            end = source.find(terminator, raw.end())
            if end < 0:
                return None
            i = end + len(terminator)
        elif source[i] == '"' or source[i:i + 2] in ('b"', 'c"'):
            literal = True
            i += 1 if source[i] == '"' else 2
            while i < len(source) and source[i] != '"':
                i += 2 if source[i] == "\\" else 1
            if i >= len(source):
                return None
            i += 1
        elif source.startswith("b'", i):
            literal = True
            end = _char_literal_end(source, i + 1)
            if end is None:
                return None
            i = end + 1
        elif (ident := raw_identifier.match(source, i)) is not None:
            i = ident.end()
        elif source[i].isdigit() and (numeric := number.match(source, i)) is not None:
            i = numeric.end()
        elif source[i] == "'" and (end := _char_literal_end(source, i)) is not None:
            literal = True
            i = end + 1
        else:
            word = word_or_operator.match(source, i)
            i = word.end() if word else i + 1
        if literal and (suffix := literal_suffix.match(source, i)) is not None:
            i = suffix.end()
        tokens.append((source[start:i], line))
        line += source[start:i].count("\n")
    return tokens


def attribute_prefix(header: tuple[str, ...]) -> tuple[str, ...]:
    """Read leading attributes without confusing nested brackets or literals."""
    end = 0
    while end + 1 < len(header) and header[end:end + 2] == ("#", "["):
        depth = 1
        end += 2
        while end < len(header) and depth:
            depth += (header[end] == "[") - (header[end] == "]")
            end += 1
        if depth:
            return ()
    return header[:end]


def module_chain(frames: list) -> tuple[str, ...]:
    return tuple(rust_identifier(frame[2][-1]) for frame in frames[1:]
                 if len(frame[2]) >= 2 and frame[2][-2] == "mod")


def is_module_scope(frames: list) -> bool:
    return all(frame[0] == "{" and len(frame[2]) >= 2 and frame[2][-2] == "mod"
               for frame in frames[1:])


def enclosing_attributes(frames: list) -> tuple:
    return tuple(value for frame in frames for value in
                 ((attribute_prefix(frame[2]) if len(frame[2]) >= 2 and frame[2][-2] == "mod" else ())
                  + tuple(value for attr in frame[4] for value in attr)))


def rust_source_units(source: str) -> tuple[list, list, dict]:
    """Return complete functions and external module attribute contexts.

    Preserve attributes and impl/trait/control-flow context. Plain inline
    modules are allowed to become separate files; attributed modules keep
    their context because e.g. changing cfg can change whether code runs.
    Nested delimiters and literals cannot end a function prematurely.
    """
    tokens = rust_move_tokens(source)
    if tokens is None:
        return [], [], {}
    units = []
    modules = []
    bindings = {}
    # Frames store opener, boundary, header, start, inner attrs, signature flag.
    frames = [["", 0, (), 0, [], False]]
    for index, (token, _) in enumerate(tokens):
        if token in ("{", "(", "["):
            start = frames[-1][1]
            header = tuple(value for value, _ in tokens[start:index])
            # A brace in a generic signature is a const expression, not
            # the function body. Keep the signature boundary across it.
            depth = 0
            brace_depth = 0
            for value in header:
                if value == "{":
                    brace_depth += 1
                elif value == "}":
                    brace_depth -= 1
                elif brace_depth == 0:
                    depth += {"<": 1, "<<": 2, ">": -1, ">>": -2}.get(value, 0)
            signature = token == "{" and "fn" in header and depth > 0
            frames.append([token, index + 1, header, start, [], signature])
        elif token in ("}", ")", "]"):
            if len(frames) == 1 or frames[-1][0] != {"}": "{", ")": "(", "]": "["}[token]:
                return [], [], {}
            opener, _, header, start, inner_attrs, signature = frames.pop()
            if opener == "[" and header == ("#", "!"):
                # Inner attributes describe the entire parent scope,
                # not just whichever item happens to appear first.
                frames[-1][4].append(tuple(value for value, _ in tokens[start:index + 1]))
                frames[-1][1] = index + 1
            if opener == "{" and not signature:
                # A definition has `fn name`, unlike a function pointer type.
                is_function = any(
                    value == "fn" and rust_identifier(following)
                    for value, following in zip(header, header[1:])
                )
                supported_scope = all(
                    frame[0] == "{" and (
                        (len(frame[2]) >= 2 and frame[2][-2] == "mod")
                        or ("fn" not in frame[2] and ("impl" in frame[2] or "trait" in frame[2]))
                    ) for frame in frames[1:]
                )
                # Nested functions can resolve local constants/imports/
                # macros. Those lexical bindings are not module bindings,
                # so do not infer preserved context for a nested function.
                if is_function and supported_scope:
                    context = []
                    for frame in frames:
                        is_module = len(frame[2]) >= 2 and frame[2][-2] == "mod"
                        if not is_module and frame[0]:
                            context.append(frame[2])
                    body = tokens[start:index + 1]
                    visible_macros = tuple((module_chain(frames)[:n], binding) for n in range(len(module_chain(frames)) + 1)
                                           for binding in bindings.get(module_chain(frames)[:n], [])
                                           if is_textual_macro(binding))
                    key = (tuple(context), enclosing_attributes(frames), tuple(value for value, _ in body), visible_macros, header)
                    token_lines = {line for value, number in body
                                   for line in range(number, number + value.count("\n") + 1)}
                    substantive = {line for value, number in body
                                   if value not in ("{", "}", "(", ")", "[", "]", ";", ",")
                                   for line in range(number, number + value.count("\n") + 1)}
                    units.append((key, token_lines, substantive, module_chain(frames)))
                if is_module_scope(frames) and len(header) >= 2 and header[-2] == "mod":
                    scope_attrs = enclosing_attributes(frames)
                    scope_attrs += attribute_prefix(header) + tuple(value for attr in inner_attrs for value in attr)
                    bindings.setdefault(module_chain(frames) + (rust_identifier(header[-1]),), []).append(("@scope",) + scope_attrs)
                    if any(rust_identifier(token) == "macro_use" for token in attribute_prefix(header)):
                        bindings.setdefault(module_chain(frames), []).append(("@macro_use", rust_identifier(header[-1])) + attribute_prefix(header))
                if is_module_scope(frames) and not (len(header) >= 2 and header[-2] == "mod"):
                    # Function signatures bind names; their bodies are
                    # checked separately by patch coverage. Other items
                    # (constants, aliases, macros, impls) retain values.
                    binding = header if is_function else tuple(value for value, _ in tokens[start:index + 1])
                    bindings.setdefault(module_chain(frames), []).append(binding)
                frames[-1][1] = index + 1
        elif token == ";":
            header = tuple(value for value, _ in tokens[frames[-1][1]:index])
            is_module = len(header) >= 2 and header[-2] == "mod" and rust_identifier(header[-1])
            if is_module and is_module_scope(frames):
                chain = module_chain(frames)
                attributes = enclosing_attributes(frames)
                attributes += attribute_prefix(header)
                visible_macros = tuple((chain[:n], binding) for n in range(len(chain) + 1)
                                       for binding in bindings.get(chain[:n], [])
                                       if is_textual_macro(binding))
                modules.append((chain, rust_identifier(header[-1]), attributes, visible_macros))
                if any(rust_identifier(token) == "macro_use" for token in attribute_prefix(header)):
                    bindings.setdefault(chain, []).append(("@macro_use", rust_identifier(header[-1])) + attribute_prefix(header))
            elif header and is_module_scope(frames):
                bindings.setdefault(module_chain(frames), []).append(header)
            frames[-1][1] = index + 1
    if len(frames) != 1:
        return [], [], {}
    bindings.setdefault((), []).append(("@scope",) + tuple(value for attr in frames[0][4] for value in attr))
    return units, modules, bindings


def module_locator(attrs: tuple) -> tuple[tuple, str | None] | None:
    """Separate a direct file locator from execution attributes.

    Conditional/ambiguous locators cannot establish a module graph, so
    their target receives no exemptions. Paths never identify a module.
    """
    retained = []
    override = None
    index = 0
    while index < len(attrs):
        start = index
        index += 1
        while index < len(attrs) and attrs[index] != "[":
            index += 1
        depth = 1
        index += 1
        while index < len(attrs) and depth:
            depth += (attrs[index] == "[") - (attrs[index] == "]")
            index += 1
        chunk = attrs[start:index]
        if "path" in chunk:
            if (override is not None or len(chunk) != 6
                    or chunk[:4] != ("#", "[", "path", "=") or chunk[-1] != "]"):
                return None
            try:
                override = json.loads(chunk[4])
            except ValueError:
                return None
            if not isinstance(override, str):
                return None
        else:
            retained.extend(chunk)
    return tuple(retained), override


def rust_module_snapshot(repo_root: str, ref: str, metadata: dict | None) -> dict:
    """Resolve source files to Cargo targets and logical module paths.

    Only actual Cargo entry files (and mod.rs) use their parent as the
    conventional child directory. Multiple target/module contexts are
    retained together; an unresolved layout fails closed for its target.
    """
    snapshot = {"sources": {}, "contexts": {}, "bindings": {}, "function_bodies": {},
                "macro_imports": {}, "build_inputs": {}, "build_data": {}, "unsafe": set(),
                "shadowed_macro_roots": {}}
    def read(path):
        if path not in snapshot["sources"]:
            source = git_show(repo_root, ref, path)
            snapshot["sources"][path] = rust_source_units(source) if source is not None else None
        return snapshot["sources"][path]

    if metadata is None:
        # The standalone helper can compare Git fixtures without Cargo.
        # Production callers always supply metadata and use the graph.
        names = subprocess.run(["git", "-C", repo_root, "ls-tree", "-r", "--name-only", ref],
                               capture_output=True, text=True, check=True).stdout.splitlines()
        for path in names:
            if path.endswith(".rs"):
                read(path)
                snapshot["contexts"][path] = {((), (), (), ())}
                for chain, values in snapshot["sources"][path][2].items():
                    snapshot["bindings"][((), chain)] = values
        return snapshot

    for package in metadata.get("packages", []):
        try:
            manifest = str(Path(package["manifest_path"]).relative_to(Path(repo_root).resolve()))
        except (KeyError, ValueError):
            continue
        for target in package.get("targets", []):
            try:
                root = str(Path(target["src_path"]).relative_to(Path(repo_root).resolve()))
            except (KeyError, ValueError):
                continue
            identity = (manifest, tuple(target.get("kind", [])), target.get("name", ""))
            snapshot["shadowed_macro_roots"][identity] = {
                dependency.get("rename") or dependency.get("name", "")
                for dependency in package.get("dependencies", [])
            } & {"std", "core", "alloc"}
            pending = [(root, (), (), (), ())]
            visited = set()
            while pending:
                path, prefix, inherited_attrs, inherited_macros, ancestors = pending.pop()
                context = (identity, prefix, inherited_attrs, inherited_macros)
                if (path, context) in visited:
                    continue
                visited.add((path, context))
                if path in ancestors:
                    snapshot["unsafe"].add(identity)
                    continue
                info = read(path)
                if info is None:
                    continue
                if "custom-build" in identity[1]:
                    tokens = rust_move_tokens(git_show(repo_root, ref, path) or "")
                    snapshot["build_inputs"][(identity, path)] = tuple(value for value, _ in tokens) if tokens is not None else None
                snapshot["contexts"].setdefault(path, set()).add(context)
                for unit in info[0]:
                    key = (identity, prefix + unit[3], unit[0][4])
                    snapshot["function_bodies"].setdefault(key, []).append(unit[0][2])
                for chain, values in info[2].items():
                    snapshot["bindings"].setdefault((identity, prefix + chain), []).extend(
                        ("@scope",) + inherited_attrs + value[1:] if value[:1] == ("@scope",) else value
                        for value in values)
                parent = PurePosixPath(path)
                directory = parent.parent if path == root or parent.name == "mod.rs" else parent.with_suffix("")
                for chain, name, attrs, macros in info[1]:
                    locator = module_locator(attrs)
                    if locator is None:
                        snapshot["unsafe"].add(identity)
                        continue
                    effective_attrs, override = locator
                    if override is not None:
                        if chain:
                            # Inline path overrides have additional Rust
                            # directory rules; do not guess those rules.
                            snapshot["unsafe"].add(identity)
                            continue
                        choices = [posixpath.normpath(str(parent.parent / override))]
                    else:
                        child = directory.joinpath(*chain, name)
                        choices = [str(child.with_suffix(".rs")), str(child / "mod.rs")]
                    children = [child for child in choices if read(child) is not None]
                    if len(children) > 1:
                        snapshot["unsafe"].add(identity)
                    elif children:
                        prefix_macros = tuple((prefix + scope, binding) for scope, binding in macros)
                        pending.append((children[0], prefix + chain + (name,),
                                        inherited_attrs + effective_attrs,
                                        inherited_macros + prefix_macros, ancestors + (path,)))
    if snapshot["build_inputs"]:
        # Build scripts can read tracked data through arbitrary helpers.
        # Their dependencies are executable compiler inputs too. Retain
        # other packages' sources as well as data outside the Rust graph.
        build_manifests = {identity[0] for identity, _ in snapshot["build_inputs"]}
        entries = subprocess.run(
            ["git", "-C", repo_root, "ls-tree", "-r", "-z", "--format=%(objectname)\t%(path)", ref],
            capture_output=True, text=True, check=True,
        ).stdout.split("\0")
        snapshot["build_data"] = {path: oid for entry in entries if entry
                                  for oid, path in [entry.split("\t", 1)]
                                  if (snapshot["sources"].get(path) is None
                                      or any(identity[0] != manifest for identity, *_ in snapshot["contexts"].get(path, ())
                                             for manifest in build_manifests))}
    # Exported macros bind at the crate root even when their definitions
    # live in sibling modules. Keep defining scope and cfg attributes.
    for (identity, scope), values in list(snapshot["bindings"].items()):
        if not scope:
            continue
        attrs = tuple(token for value in values if value[:1] == ("@scope",) for token in value[1:])
        for binding in values:
            if any(rust_identifier(token) == "macro_export" for token in attribute_prefix(binding)):
                snapshot["bindings"].setdefault((identity, ()), []).append(("@exported_macro",) + scope + attrs + binding)
    # Imported legacy macros retain declaration order and definition
    # contents. Resolve deepest modules first for nested macro_use imports.
    for (identity, scope), values in sorted(snapshot["bindings"].items(), key=lambda item: len(item[0][1]), reverse=True):
        resolved = []
        for binding in values:
            if binding[:1] == ("@macro_use",):
                raw_binding = binding
                child = snapshot["bindings"].get((identity, scope + (binding[1],)))
                locator = module_locator(binding[2:])
                if child is None or locator is None:
                    binding = ("@macro_use_unknown",)
                else:
                    macros = [value for value in child if is_textual_macro(value)]
                    binding = ("@macro_use", binding[1]) + locator[0] + tuple(token for value in macros for token in ("@definition",) + value)
                snapshot["macro_imports"][(identity, scope, raw_binding)] = binding
            resolved.append(binding)
        snapshot["bindings"][(identity, scope)] = resolved
    return snapshot


def binding_names(tokens: tuple) -> set[str] | None:
    """Identify names introduced by common module-level declarations."""
    if tokens[:1] in (("@macro_use",), ("@macro_use_unknown",)):
        return None
    kinds = {"fn", "const", "static", "type", "struct", "enum", "trait", "macro_rules", "macro"}
    for index, token in enumerate(tokens):
        if token == "use":
            # Trait imports affect method lookup without the trait name
            # appearing in the body, including imports aliased as `_`.
            return None
        if token in kinds:
            return {name for value in tokens[index + 1:] if (name := rust_identifier(value)) is not None}
        if token == "impl":
            return None
    # Unknown expansions can introduce any referenced name.
    return None


def has_file_include(tokens: tuple) -> bool:
    includes = {"include", "include_str", "include_bytes"}
    # A built-in may be aliased, re-exported or passed to another macro.
    # Unknown expansion cannot establish unchanged file/span inputs. Literal
    # text remains quoted tokens and does not match these identifiers.
    if any(rust_identifier(token) in includes for token in tokens):
        return True
    # Location built-ins can be imported or passed to another macro.
    # Ordinary values named file/line/column remain valid move candidates.
    delimiters = []
    in_import = False
    for index, token in enumerate(tokens):
        if token == "use":
            in_import = True
        elif token == ";":
            in_import = False
        if rust_identifier(token) in ("file", "line", "column"):
            if in_import or any(delimiters) or tokens[index + 1:index + 2] == ("!",):
                return True
        if token in ("(", "[", "{"):
            delimiters.append(any(delimiters) or (index > 0 and tokens[index - 1] == "!"))
        elif token in (")", "]", "}") and delimiters:
            delimiters.pop()
    return False


def has_runtime_file_input(tokens: tuple) -> bool:
    """Reject build-script file reads whose paths cannot be established.

    A script may read modeled Rust sources as data. Their relocation also
    changes input bytes, so comparing only the remaining files is unsafe.
    Include aliases and common handle/reader APIs conservatively.
    """
    names = {"fs", "File", "OpenOptions", "BufReader", "BufRead", "Read", "read",
             "read_to_string", "read_to_end", "read_exact", "read_dir", "metadata"}
    return any(rust_identifier(token) in names for token in tokens)


def is_textual_macro(tokens: tuple) -> bool:
    return (tokens[:1] in (("@macro_use",), ("@macro_use_unknown",))
            or any(rust_identifier(token) in ("macro_rules", "macro") for token in tokens))


def has_opaque_expansion(tokens: tuple, local_names: set[str], shadowed_roots: set[str]) -> bool:
    """Unknown compiler extensions may read files without a build script.

    Their source tokens do not establish unchanged expansion inputs. Keep
    modeled declarative macros and standard macros eligible, but require
    coverage for functions reaching opaque invocations or attributes.
    """
    standard = {"assert", "assert_eq", "assert_ne", "cfg", "concat", "concat_idents",
                "debug_assert", "debug_assert_eq", "debug_assert_ne", "eprint", "eprintln",
                "format", "format_args", "matches", "module_path", "panic", "print", "println",
                "stringify", "todo", "unimplemented", "unreachable", "vec", "write", "writeln"}
    attributes = {"allow", "warn", "deny", "forbid", "expect", "cfg", "doc", "inline",
                  "cold", "must_use", "deprecated", "repr", "path", "macro_use", "macro_export",
                  "test", "ignore", "should_panic", "no_mangle", "export_name", "link_name",
                  "link", "link_section", "non_exhaustive", "no_std", "no_main", "crate_type",
                  "crate_name", "recursion_limit", "type_length_limit", "feature"}
    if (any(rust_identifier(token) == "macro_use" for token in attribute_prefix(tokens))
            and any(pair == ("extern", "crate") for pair in zip(tokens, tokens[1:]))):
        return True
    imports = []
    for start, value in enumerate(tokens):
        if value == "use":
            end = next((n for n in range(start, len(tokens)) if tokens[n] == ";"), len(tokens))
            imported = tuple(rust_identifier(token) or token for token in tokens[start + 1:end])
            imports.append(imported[1:] if imported[:1] == ("::",) else imported)
    shadowed_roots = shadowed_roots | {
        root for root in ("std", "core", "alloc")
        if any("*" in value or root in value[1:] for value in imports)
        or any(token in {"mod", "struct", "enum", "type", "trait", "as"}
               and rust_identifier(following) == root for token, following in zip(tokens, tokens[1:]))
    }
    for index, token in enumerate(tokens):
        if token == "#":
            start = index + (2 if tokens[index + 1:index + 2] == ("!",) else 1)
            if tokens[start:start + 1] == ("[",):
                name = rust_identifier(tokens[start + 1]) if start + 1 < len(tokens) else None
                if name not in attributes or tokens[start + 2:start + 3] == ("::",):
                    return True
        name = rust_identifier(token)
        if name is None or tokens[index + 1:index + 2] != ("!",) or tokens[index + 2:index + 3] not in (("(",), ("[",), ("{",)):
            continue
        # Keywords before unary negation are not macro paths.
        if name in {"return", "if", "while", "match", "else", "break", "yield", "in"}:
            continue
        qualified = index > 0 and tokens[index - 1] == "::"
        if not qualified and name in local_names and not any("*" in value or name in value for value in imports):
            continue
        if name in standard:
            if qualified:
                root = rust_identifier(tokens[index - 2]) if index >= 2 else None
                if (root in {"std", "core", "alloc"} and root not in shadowed_roots
                        and (index < 3 or tokens[index - 3] != "::" or index == 3)):
                    continue
            else:
                # An explicit or glob import can shadow a prelude macro.
                if not any("*" in value or name in value for value in imports):
                    continue
        return True
    return False


def function_move_key(snapshot: dict, path: str, unit: tuple) -> tuple | None:
    """Compare execution context, including bindings used by a function.

    Follow references through constants, imports, signatures and aliases.
    Qualified module references also include the corresponding scopes.
    Declaration order is irrelevant except for textual macro visibility.
    """
    (runtime, attrs, body, local_macros, _), _, _, local_chain = unit
    if has_file_include(body):
        return None
    contexts = snapshot["contexts"].get(path)
    if not contexts:
        return None
    keys = []
    for identity, prefix, inherited_attrs, inherited_macros in contexts:
        if identity in snapshot["unsafe"]:
            return None
        logical = prefix + local_chain
        references = {name for token in body if (name := rust_identifier(token)) is not None}
        macro_prefix = inherited_macros + tuple((prefix + scope, binding) for scope, binding in local_macros)
        relevant = set()
        changed = True
        while changed:
            previous = len(references)
            for (target, scope), bindings in snapshot["bindings"].items():
                if target != identity:
                    continue
                if not (logical[:len(scope)] == scope or any(name in references for name in scope)):
                    continue
                for binding in bindings:
                    bodies = snapshot["function_bodies"].get((identity, scope, binding), [])
                    if bodies:
                        # A function binds its name, not every type named
                        # in its signature. Retain referenced bodies and
                        # follow their references through helper calls.
                        header = binding[len(attribute_prefix(binding)):]
                        names = {name for token, following in zip(header, header[1:])
                                 if token == "fn" and (name := rust_identifier(following)) is not None}
                    else:
                        names = binding_names(binding)
                    if names is None or references & names:
                        # Expansion inputs can change bindings or values
                        # outside this graph. Fail closed only for functions
                        # whose scope/references reach those inputs.
                        if "@macro_use_unknown" in binding or has_file_include(binding) or any(has_file_include(value) for value in bodies):
                            return None
                        relevant.add((scope, binding))
                        references.update(name for token in binding if (name := rust_identifier(token)) is not None)
                        for value in bodies:
                            relevant.add((scope, ("@function_body",) + value))
                            references.update(name for token in value if (name := rust_identifier(token)) is not None)
            changed = len(references) != previous
        macros = []
        for scope, binding in macro_prefix:
            if binding[:1] == ("@macro_use",):
                binding = snapshot["macro_imports"].get((identity, scope, binding))
                if binding is None or "@macro_use_unknown" in binding:
                    return None
            if binding_names(binding) is None or references & binding_names(binding):
                macros.append((scope, binding))
        macros = tuple(macros)
        inputs = [body, inherited_attrs + attrs, *(binding for _, binding in relevant),
                  *(binding for _, binding in macros)]
        local_names = set()
        for tokens in inputs:
            for index, token in enumerate(tokens):
                offset = (2 if rust_identifier(token) == "macro_rules" and tokens[index + 1:index + 2] == ("!",)
                          else 1 if token == "macro" else 0)
                if offset and index + offset < len(tokens):
                    name = rust_identifier(tokens[index + offset])
                    if name is not None:
                        local_names.add(name)
        # Include imports when checking unqualified standard macros: a
        # dependency can export a macro with the same prelude name.
        imports = tuple(token for _, binding in relevant if "use" in binding for token in binding)
        shadowed_roots = snapshot["shadowed_macro_roots"].get(identity, set()) | {
            name for (target, scope) in snapshot["bindings"] if target == identity
            for name in scope if name in {"std", "core", "alloc"}
        } | {
            name for tokens in inputs for token, following in zip(tokens, tokens[1:])
            if token in {"mod", "struct", "enum", "type", "trait", "as"}
            and (name := rust_identifier(following)) in {"std", "core", "alloc"}
        }
        if any(has_opaque_expansion(tokens + imports, local_names, shadowed_roots) for tokens in inputs):
            return None
        keys.append((identity, logical, inherited_attrs + attrs, tuple(sorted(relevant)), macros))
    return tuple(sorted(keys)), runtime, body


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

    try:
        metadata = load_cargo_metadata(args.repo_root)
    except CargoMetadataError as exc:
        print(f"patch_coverage.py: {exc}", file=sys.stderr)
        return 2
    unreported_paths = structurally_unreported_paths(metadata, args.repo_root)

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
    moved_by_file = collect_moved_added_lines(args.repo_root, args.base, args.head, metadata)

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
        moved = moved_by_file.get(path, set())

        da_map = lcov_by_file.get(path)
        if da_map is None:
            fr.no_lcov_record = True
            if is_structurally_unreported(path, unreported_paths):
                # A Cargo test/example target's entry file, or a
                # `tests.rs`-named module file: cargo-llvm-cov never lists
                # an SF: record for these regardless of whether they ran —
                # see is_structurally_unreported(). Not a coverage failure.
                fr.unreported.extend(fr.added_lines)
            else:
                # benches/*.rs (or any other file cargo-llvm-cov's target
                # set didn't build/run at all, e.g. a zero-executable-line
                # "pure declaration" src file): added *code* lines get the
                # same treatment as an uncovered line. Blank/comment added
                # lines are filtered out first, mirroring the `if ln not
                # in da_map: continue` skip the branch below does with an
                # actual DA: table — see classify_no_lcov_record_lines().
                uncovered, exempted = classify_no_lcov_record_lines(
                    fr.added_lines, file_lines, exempt
                )
                fr.uncovered.extend(ln for ln in uncovered if ln not in moved)
                fr.moved.extend(ln for ln in uncovered if ln in moved)
                fr.exempted.extend(exempted)
        else:
            for ln in fr.added_lines:
                if ln not in da_map:
                    continue  # not an instrumented/executable line
                if da_map[ln] > 0:
                    continue  # covered
                if ln in exempt:
                    fr.exempted.append(ln)
                elif ln in moved:
                    fr.moved.append(ln)
                else:
                    fr.uncovered.append(ln)
        results.append(fr)

    total_added = sum(len(r.added_lines) for r in results)
    total_uncovered = sum(len(r.uncovered) for r in results)
    total_exempted = sum(len(r.exempted) for r in results)
    total_unreported = sum(len(r.unreported) for r in results)
    total_moved = sum(len(r.moved) for r in results)

    print(f"patch coverage: base={args.base} head={args.head}")
    print(f"changed .rs files with added lines: {len(results)}")
    print(f"total added lines (diff): {total_added}")
    print(f"total cov:ignore-exempted added lines: {total_exempted}")
    print(f"total unreported added lines (test/example targets, tests.rs files; informational): {total_unreported}")
    print(f"total moved uncovered lines (moved by this diff, not new; informational): {total_moved}")
    print(f"total uncovered added lines (FAIL if > 0): {total_uncovered}")
    print()

    if total_moved:
        print("Moved uncovered lines per file — complete unchanged functions")
        print("deleted elsewhere in this diff support these move exemptions; not")
        print("counted toward PASS/FAIL:")
        for r in results:
            if r.moved:
                print(f"  {r.path}: {len(r.moved)}")
        print()

    if total_unreported:
        print("Unreported changed lines (file:line) — cargo-llvm-cov does not")
        print("list an SF: record for a Cargo test/example target's entry file or a")
        print("tests.rs-named file regardless of whether the test ran; not counted")
        print("toward PASS/FAIL:")
        for r in results:
            for ln in r.unreported:
                print(f"  {r.path}:{ln}")
        print()

    if total_uncovered == 0:
        if total_unreported:
            print(
                "PASS: all changed lines are covered or cov:ignore-exempted, "
                f"except {total_unreported} line(s) in Cargo test/example "
                "target entry files or tests.rs-named files that "
                "cargo-llvm-cov does not report on "
                "(see 'Unreported changed lines' above) — those are not "
                "verified covered, only not counted as a failure."
            )
        else:
            print("PASS: all changed lines are covered or cov:ignore-exempted.")
        if total_moved:
            print(
                f"({total_moved} uncovered line(s) were only moved by this diff; "
                "see 'Moved uncovered lines' above.)"
            )
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
    print("escalate to a follow-up item (out-of-scope) for each line above, or add")
    print("`// cov:ignore: <reason>` if it is a defensible untestable branch.")
    return 1


if __name__ == "__main__":
    sys.exit(main())
