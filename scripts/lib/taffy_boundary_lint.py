#!/usr/bin/env python3
"""scripts/lib/taffy_boundary_lint.py — Taffy output-type boundary ratchet.

## Why this exists

raikiri-dom lays boxes out by calling Taffy, and the layout results, the layout
call types and the node handle that Taffy defines have leaked into every layer:
the paint walk, the page pipeline, the inline engine, tables and multicol all
name `taffy::Layout`, `taffy::Size`, `taffy::LayoutInput` and so on directly.
The migration to types owned by this repository (a dependency-free geometry
crate plus a call-protocol module inside raikiri-dom) is done in two steps:
first every import path is switched to the owned names while the owned names
are still aliases of the Taffy types, then the definitions are replaced in one
change. This lint keeps the first step from going backwards while it is in
flight, and fixes the end state afterwards: no Taffy output-type name outside
the adapter and the view crate.

## What is counted

Only the "output side" names in `OUTPUT_NAMES`: geometry, layout results, the
layout call protocol, the node handle, the cache, and the detailed layout info
that is handed straight back to Taffy. Input-side vocabulary (`Dimension`,
`LengthPercentage`, `CoreStyle`, `Style`, `Display`, ...) is deliberately not
counted.

An occurrence is a *path* that starts at the `taffy` crate and names one of
those types:

  - one per leaf of a `use taffy::...` tree (nested groups and `as` aliases are
    expanded; the occurrence sits on the line of the leaf, so a multi-line
    import is counted per item), e.g. `use taffy::{Size, tree::LayoutInput};`
    is two occurrences;
  - one per inline `taffy::...::Name` path in code, e.g. `taffy::NodeId::from(i)`;
  - one per glob import of a Taffy module (`use taffy::tree::*;`), because a
    glob can import any of the names;
  - one per import of the crate or of one of its modules (`use taffy as t;`,
    `use taffy::tree;`, `use taffy::geometry::{self}`), because the names are
    then reached as `t::Size` or `tree::NodeId`, which a path scan cannot tie
    back to Taffy. Import the type itself instead.

A path names a type when its first capitalised segment after `taffy` is one of
the counted names, so `taffy::Size::ZERO` and `taffy::tree::LayoutInput` count,
while `taffy::GridPlacement::Line(1)` (an input-side enum variant that happens
to be called `Line`) does not.

Comments and string literals are ignored, and only `.rs` files under `crates/`
are scanned (not `crates/*/target`). Names that arrive through an in-crate
alias are not seen at the use site: the alias definition is the counted
occurrence, and swapping it swaps every user.

`Size`, `Rect`, `Point` and `Line` are generic. They also appear on the input
side (`Size<Dimension>`, `Rect<LengthPercentageAuto>`, `Line<GridPlacement>`),
and a path cannot tell the two apart without type information. They are counted
regardless, so the number is an upper bound. A site that really is input-side
(or otherwise has to keep the Taffy name) is marked on the same line:

    use taffy::Size as StyleSize; // taffy-boundary-lint:ignore: Size<Dimension> is style input

The reason after the second colon is required; a marker without one is not
honoured. A marked occurrence is neither counted nor baselined; the summary
shows how many there are (`-v` lists them) so the exceptions stay visible.

## Where it is not counted

`taffy_boundary_exempt.txt` lists path prefixes (a trailing `/` means a
directory) that may name Taffy types freely: the view crate, the geometry crate
and the protocol module that re-export the Taffy types, and the adapter once it
exists. Everything else is held to the ratchet.

The owned names shown in the failure output come from two places. Geometry and
the box layout result live in the `raikiri-layout-geometry` crate and can be
used from any crate. The layout call protocol lives in raikiri-dom's
crate-private `layout::protocol` module (nothing outside raikiri-dom reads a
layout input or output), so a use of those names in another crate has no owned
name to move to and stays pinned until the raikiri-dom API that replaces it
exists.

Some counted names have no owned replacement because they are handed straight
back to Taffy and belong in the adapter: `Cache`, `ClearState`,
`DetailedGridInfo`, `DetailedGridTracksInfo`, `DetailedGridItemsInfo`,
`DetailedLayoutInfo` and `OofLayoutResult` (shown as "(adapter only)" in the
failure output). Their uses stay pinned in the baseline until the adapter module
exists and takes them; they are not a reason to add an ignore marker.

The tree-trait implementations and the calls into Taffy's algorithms will end
up in the adapter, but they are not exempt yet. The import-path step switches
them too (the types are still identical, so the code compiles unchanged), and
the adapter module is added to the exempt list when the definitions are
replaced.

## The ratchet

`taffy_boundary_baseline.txt` pins the current count per file. A run fails when

  - a file has more occurrences than its baseline (or any, if it has no entry):
    new uses of a Taffy output type;
  - a file has fewer occurrences than its baseline, or a baseline entry names a
    file that no longer exists or is now exempt: the pin is stale, and a stale
    pin would let later additions hide behind the slack.

Pinning per file instead of one total keeps parallel import-path changes from
conflicting on a single number, and names the file that regressed.
`--update-baseline` rewrites the file from the current counts, but only
downwards: it lowers a line, drops a line that reached zero, and refuses to run
when any file has more occurrences than its pin or has none (a moved file, or
new uses). Rename the line of a moved file by hand; the baseline diff then shows
the move for review.

Usage: `python3 taffy_boundary_lint.py --repo-root PATH [-v] [--print-counts]
[--update-baseline]`
Exit status: 0 on pass, 1 on a ratchet failure, 2 on a tooling error (unreadable
baseline, source that cannot be tokenised).
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
DEFAULT_BASELINE = SCRIPT_DIR / "taffy_boundary_baseline.txt"
DEFAULT_EXEMPT = SCRIPT_DIR / "taffy_boundary_exempt.txt"

# The Taffy names this lint counts, with the owned name each one is moving to.
# The owned names live in the `raikiri-layout-geometry` crate (geometry and the
# box layout result) and in raikiri-dom's `layout::protocol` module (the call
# protocol). Names without an owned replacement are adapter-only: they are
# handed straight back to Taffy and stay in the adapter.
OUTPUT_NAMES: dict[str, str] = {
    "Size": "raikiri_layout_geometry::Size",
    "Point": "raikiri_layout_geometry::Point",
    "Line": "raikiri_layout_geometry::Line",
    "Rect": "raikiri_layout_geometry::Edges",
    "Layout": "raikiri_layout_geometry::BoxLayout",
    "AbsoluteAxis": "crate::layout::protocol::AbsoluteAxis",
    "AvailableSpace": "crate::layout::protocol::AvailableSpace",
    "LayoutInput": "crate::layout::protocol::LayoutInput",
    "LayoutOutput": "crate::layout::protocol::LayoutOutput",
    "RunMode": "crate::layout::protocol::RunMode",
    "SizingMode": "crate::layout::protocol::SizingMode",
    "RequestedAxis": "crate::layout::protocol::RequestedAxis",
    "CollapsibleMarginSet": "crate::layout::protocol::CollapsibleMarginSet",
    "Baselines": "crate::layout::protocol::Baselines",
    "OofCandidate": "crate::layout::protocol::OofCandidate",
    "OofCandidates": "crate::layout::protocol::OofCandidates",
    "OofPositioningArea": "crate::layout::protocol::OofPositioningArea",
    "AxisStaticEdge": "crate::layout::protocol::AxisStaticEdge",
    "AxisStaticAlign": "crate::layout::protocol::AxisStaticAlign",
    "AxisStaticPosition": "crate::layout::protocol::AxisStaticPosition",
    "NodeId": "crate::layout::protocol::NodeId",
    "Cache": "(adapter only)",
    "ClearState": "(adapter only)",
    "DetailedGridInfo": "(adapter only)",
    "DetailedGridTracksInfo": "(adapter only)",
    "DetailedGridItemsInfo": "(adapter only)",
    "DetailedLayoutInfo": "(adapter only)",
    "OofLayoutResult": "(adapter only)",
}

# A failing file lists at most this many occurrences; -v lists all of them.
MAX_LISTED_PER_FILE = 12

# Taffy's public modules. Importing one of them (or the crate itself) lets the
# output types be written as `tree::NodeId`, which the path scan cannot see.
TAFFY_MODULES = frozenset({"compute", "geometry", "prelude", "style", "style_helpers", "tree", "util"})

# Pseudo names for the occurrences that are not a specific output type.
GLOB_IMPORT = "*"
MODULE_IMPORT = "<module>"

IGNORE_MARKER_RE = re.compile(r"//\s*taffy-boundary-lint:ignore:\s*(\S.*)$")
IGNORE_MARKER_WITHOUT_REASON_RE = re.compile(r"//\s*taffy-boundary-lint:ignore:\s*$")

_IDENT_RE = re.compile(r"(?:r#)?[A-Za-z_][A-Za-z0-9_]*")
_USE_RE = re.compile(r"(?<![\w$])use(?=\s+(?:[A-Za-z_$]|\{|::|\*))")
_TAFFY_PATH_RE = re.compile(r"(?<![\w:])(?:::\s*)?taffy(?!\w)\s*::")
_USE_TOKEN_RE = re.compile(r"::|(?:r#)?[A-Za-z_][A-Za-z0-9_]*|[{},*;]|\S")


class TokenizeError(Exception):
    """Source that this scanner cannot read (unterminated literal or comment, malformed `use`)."""


@dataclass(frozen=True)
class Occurrence:
    path: str
    line: int
    name: str
    text: str

    def __str__(self) -> str:
        return f"{self.path}:{self.line}: {self.text}"


@dataclass(frozen=True)
class FileScan:
    counted: tuple[Occurrence, ...]
    ignored: tuple[Occurrence, ...]
    unreasoned_markers: tuple[int, ...]


def strip_comments_and_strings(src: str) -> str:
    """Blank out comments, string literals and char literals, keeping every newline.

    The result has the same length and the same line structure as `src`, so an
    offset in it maps to the same line in the original. Lifetimes and labels
    (`'a`, `'outer:`) are left alone; only real char literals are blanked.
    """
    out = list(src)
    n = len(src)
    i = 0

    def blank(start: int, end: int) -> None:
        for k in range(start, end):
            if out[k] != "\n":
                out[k] = " "

    while i < n:
        c = src[i]
        two = src[i : i + 2]
        if two == "//":
            j = src.find("\n", i)
            j = n if j == -1 else j
            blank(i, j)
            i = j
        elif two == "/*":
            depth = 1
            j = i + 2
            while j < n and depth:
                if src[j : j + 2] == "/*":
                    depth += 1
                    j += 2
                elif src[j : j + 2] == "*/":
                    depth -= 1
                    j += 2
                else:
                    j += 1
            if depth:
                raise TokenizeError("unterminated block comment")
            blank(i, j)
            i = j
        elif c == '"' or (c in "rbc" and _starts_string(src, i)):
            j = _end_of_string(src, i)
            blank(i, j)
            i = j
        elif c == "'":
            j = _end_of_char_literal(src, i)
            if j is None:
                i += 1  # a lifetime or label
            else:
                blank(i, j)
                i = j
        else:
            i += 1
    return "".join(out)


def _starts_string(src: str, i: int) -> bool:
    """Whether a prefixed string literal (`r"`, `r#"`, `b"`, `br"`, `c"`, `cr#"`) starts at `i`."""
    if i > 0 and (src[i - 1].isalnum() or src[i - 1] == "_"):
        return False
    j = i
    if src[j] in "bc":
        j += 1
    if j < len(src) and src[j] == "r":
        j += 1
        while j < len(src) and src[j] == "#":
            j += 1
    return j < len(src) and src[j] == '"' and j > i


def _end_of_string(src: str, i: int) -> int:
    """Offset one past the string literal that starts at `i` (with any prefix)."""
    j = i
    while src[j] in "bcr":
        j += 1
    raw = "r" in src[i:j]
    hashes = 0
    while src[j] == "#":
        hashes += 1
        j += 1
    j += 1  # the opening quote
    if raw:
        closing = '"' + "#" * hashes
        k = src.find(closing, j)
        if k == -1:
            raise TokenizeError("unterminated raw string literal")
        return k + len(closing)
    while j < len(src):
        if src[j] == "\\":
            j += 2
        elif src[j] == '"':
            return j + 1
        else:
            j += 1
    raise TokenizeError("unterminated string literal")


def _end_of_char_literal(src: str, i: int) -> int | None:
    """Offset one past the char literal at `i`, or None if the quote starts a lifetime."""
    n = len(src)
    if i + 1 >= n:
        return None
    if src[i + 1] == "\\":
        j = i + 3  # skip the backslash and the escaped character
        while j < n and src[j] != "'":
            if src[j] == "\n":
                return None
            j += 1
        return j + 1 if j < n else None
    if i + 2 < n and src[i + 2] == "'" and src[i + 1] != "'":
        return i + 3
    return None


@dataclass(frozen=True)
class _Tok:
    text: str
    line: int
    pos: int


def _tokenize_use(stripped: str, start: int, base_line: int) -> tuple[list[_Tok], int]:
    """Tokens of the `use` item whose body starts at `start`, and the offset after its `;`."""
    end = stripped.find(";", start)
    if end == -1:
        raise TokenizeError(f"line {base_line}: `use` without a terminating `;`")
    toks = []
    for m in _USE_TOKEN_RE.finditer(stripped, start, end):
        line = base_line + stripped.count("\n", start, m.start())
        toks.append(_Tok(m.group(0).removeprefix("r#"), line, m.start()))
    return toks, end + 1


@dataclass(frozen=True)
class UseLeaf:
    segments: tuple[str, ...]
    line: int
    pos: int
    glob: bool


def parse_use_tree(toks: list[_Tok]) -> list[UseLeaf]:
    """Expand a use tree into one leaf per imported path (a glob is a leaf with `glob=True`)."""
    leaves: list[UseLeaf] = []
    pos = _parse_tree(toks, 0, (), leaves)
    if pos != len(toks):
        raise TokenizeError(f"line {toks[pos].line}: unexpected `{toks[pos].text}` in a `use` tree")
    return leaves


def _parse_tree(toks: list[_Tok], i: int, prefix: tuple[str, ...], out: list[UseLeaf]) -> int:
    segs = list(prefix)
    if i < len(toks) and toks[i].text == "::":
        i += 1
    while True:
        if i >= len(toks):
            raise TokenizeError("a `use` tree ends in the middle of a path")
        t = toks[i]
        if t.text == "{":
            i += 1
            while i < len(toks) and toks[i].text != "}":
                i = _parse_tree(toks, i, tuple(segs), out)
                if i < len(toks) and toks[i].text == ",":
                    i += 1
            if i >= len(toks):
                raise TokenizeError(f"line {t.line}: unbalanced `{{` in a `use` tree")
            return i + 1
        if t.text == "*":
            out.append(UseLeaf(tuple(segs), t.line, t.pos, True))
            return i + 1
        if _IDENT_RE.fullmatch(t.text):
            segs.append(t.text)
            i += 1
            if i < len(toks) and toks[i].text == "::":
                i += 1
                continue
            if i + 1 < len(toks) and toks[i].text == "as":
                i += 2
            out.append(UseLeaf(tuple(segs), t.line, t.pos, False))
            return i
        raise TokenizeError(f"line {t.line}: unexpected `{t.text}` in a `use` tree")


def _output_name(segments: tuple[str, ...]) -> str | None:
    """The counted type a path starting at `taffy` names, if any.

    Modules are lower case, so the first capitalised segment is the type (or
    the enum whose variant is meant); only that segment is looked up.
    """
    for seg in segments[1:]:
        if seg[:1].isupper():
            return seg if seg in OUTPUT_NAMES else None
    return None


def scan_source(rel_path: str, src: str) -> FileScan:
    """Count the Taffy output-type occurrences in one Rust source file."""
    stripped = strip_comments_and_strings(src)
    src_lines = src.split("\n")

    raw_hits: list[tuple[int, int, str, str]] = []  # (offset, line, name, text)
    blanked = list(stripped)

    for m in _USE_RE.finditer(stripped):
        line = stripped.count("\n", 0, m.start()) + 1
        try:
            toks, end = _tokenize_use(stripped, m.end(), line)
            leaves = parse_use_tree(toks)
        except TokenizeError:
            # Macro fragments (`use $crate::x;`) and the like. Skipping is only
            # safe when the item cannot be a Taffy import.
            if "taffy" in stripped[m.end() : m.end() + 200].split(";", 1)[0]:
                raise
            continue
        for k in range(m.start(), end):
            if blanked[k] != "\n":
                blanked[k] = " "
        for leaf in leaves:
            if not leaf.segments or leaf.segments[0] != "taffy":
                continue
            if leaf.glob:
                text = "use " + "::".join(leaf.segments) + "::*"
                raw_hits.append((leaf.pos, leaf.line, GLOB_IMPORT, text))
                continue
            # `use taffy::tree::{self}` imports the module `tree`.
            module = leaf.segments[:-1] if leaf.segments[-1] == "self" else leaf.segments
            if module == ("taffy",) or module[-1] in TAFFY_MODULES:
                raw_hits.append((leaf.pos, leaf.line, MODULE_IMPORT, "use " + "::".join(module)))
                continue
            name = _output_name(leaf.segments)
            if name is not None:
                raw_hits.append((leaf.pos, leaf.line, name, "use " + "::".join(leaf.segments)))

    inline = "".join(blanked)
    for m in _TAFFY_PATH_RE.finditer(inline):
        segs = ["taffy"]
        pos = m.end()
        while True:
            ident = _IDENT_RE.match(inline, _skip_space(inline, pos))
            if ident is None:
                break
            segs.append(ident.group(0).removeprefix("r#"))
            after = _skip_space(inline, ident.end())
            if inline.startswith("::", after) and _IDENT_RE.match(inline, _skip_space(inline, after + 2)):
                pos = after + 2
                continue
            break
        name = _output_name(tuple(segs))
        if name is not None:
            line = inline.count("\n", 0, m.start()) + 1
            raw_hits.append((m.start(), line, name, "::".join(segs)))

    counted: list[Occurrence] = []
    ignored: list[Occurrence] = []
    unreasoned: list[int] = []
    for _pos, line, name, text in sorted(raw_hits):
        raw_line = src_lines[line - 1] if line - 1 < len(src_lines) else ""
        occ = Occurrence(rel_path, line, name, text)
        if IGNORE_MARKER_RE.search(raw_line):
            ignored.append(occ)
        else:
            if IGNORE_MARKER_WITHOUT_REASON_RE.search(raw_line):
                unreasoned.append(line)
            counted.append(occ)
    return FileScan(tuple(counted), tuple(ignored), tuple(unreasoned))


def _skip_space(text: str, pos: int) -> int:
    while pos < len(text) and text[pos].isspace():
        pos += 1
    return pos


def load_exempt(exempt_file: Path) -> list[str]:
    """Path prefixes from the exempt list: one per line, `#` starts a comment."""
    prefixes = []
    for raw in exempt_file.read_text(encoding="utf-8").splitlines():
        entry = raw.split("#", 1)[0].strip()
        if entry:
            prefixes.append(entry)
    return prefixes


def is_exempt(rel_path: str, prefixes: list[str]) -> bool:
    return any(rel_path == p or (p.endswith("/") and rel_path.startswith(p)) for p in prefixes)


def load_baseline(baseline_file: Path) -> dict[str, int]:
    """`<count> <path>` per line, `#` starts a comment. Raises ValueError on a bad line."""
    baseline: dict[str, int] = {}
    for n, raw in enumerate(baseline_file.read_text(encoding="utf-8").splitlines(), start=1):
        entry = raw.split("#", 1)[0].strip()
        if not entry:
            continue
        parts = entry.split(None, 1)
        if len(parts) != 2 or not parts[0].isdigit() or int(parts[0]) == 0:
            raise ValueError(f"{baseline_file}:{n}: expected `<positive count> <path>`, got {raw!r}")
        if parts[1] in baseline:
            raise ValueError(f"{baseline_file}:{n}: duplicate entry for {parts[1]}")
        baseline[parts[1]] = int(parts[0])
    return baseline


def source_files(repo_root: Path) -> list[Path]:
    """Every Rust file under `crates/`, except a crate's own `target/` build output."""
    return sorted(
        p
        for p in (repo_root / "crates").rglob("*.rs")
        if p.relative_to(repo_root).parts[2:3] != ("target",)
    )


@dataclass
class Census:
    counts: dict[str, int]
    occurrences: dict[str, tuple[Occurrence, ...]]
    ignored: list[Occurrence]
    unreasoned: list[tuple[str, int]]
    exempt_files: int


def take_census(repo_root: Path, exempt: list[str]) -> Census:
    census = Census({}, {}, [], [], 0)
    for path in source_files(repo_root):
        rel = path.relative_to(repo_root).as_posix()
        if is_exempt(rel, exempt):
            census.exempt_files += 1
            continue
        try:
            scan = scan_source(rel, path.read_text(encoding="utf-8"))
        except (TokenizeError, UnicodeDecodeError) as err:
            raise TokenizeError(f"{rel}: {err}") from err
        census.ignored.extend(scan.ignored)
        census.unreasoned.extend((rel, line) for line in scan.unreasoned_markers)
        if scan.counted:
            census.counts[rel] = len(scan.counted)
            census.occurrences[rel] = scan.counted
    return census


@dataclass
class Verdict:
    more: list[str]
    fewer: list[str]
    gone: list[str]

    @property
    def ok(self) -> bool:
        return not (self.more or self.fewer or self.gone)


def compare(census: Census, baseline: dict[str, int]) -> Verdict:
    verdict = Verdict([], [], [])
    for path, count in sorted(census.counts.items()):
        pinned = baseline.get(path, 0)
        if count > pinned:
            verdict.more.append(path)
        elif count < pinned:
            verdict.fewer.append(path)
    # The baseline never holds a zero, so an entry without a counted file is
    # a deleted, moved, exempted or fully converted file.
    verdict.gone.extend(path for path in sorted(baseline) if path not in census.counts)
    return verdict


def format_baseline(counts: dict[str, int], header: str) -> str:
    lines = [header.rstrip("\n"), ""]
    lines.extend(f"{count} {path}" for path, count in sorted(counts.items()))
    return "\n".join(lines) + "\n"


def read_header(baseline_file: Path) -> str:
    """The leading comment block of the baseline file, kept verbatim on rewrite."""
    header = []
    for raw in baseline_file.read_text(encoding="utf-8").splitlines():
        if raw.startswith("#"):
            header.append(raw)
        else:
            break
    return "\n".join(header)


def _replacement(name: str, path: str) -> str:
    if name == GLOB_IMPORT:
        return "import the owned names explicitly"
    if name == MODULE_IMPORT:
        return "import the type by its full path, or its owned name"
    owned = OUTPUT_NAMES[name]
    if owned.startswith("crate::layout::protocol::") and not path.startswith("crates/raikiri-dom/"):
        return "no owned name outside raikiri-dom yet"
    return owned


def _group(path: str) -> str:
    parts = path.split("/")
    return "/".join(parts[:2]) if len(parts) > 2 else path


def _print_summary(census: Census, baseline: dict[str, int]) -> None:
    total = sum(census.counts.values())
    print(f"  counted occurrences      : {total} in {len(census.counts)} file(s)")
    print(f"  pinned in baseline       : {sum(baseline.values())} in {len(baseline)} file(s)")
    print(f"  exempt files not scanned : {census.exempt_files}")
    print(f"  ignored by marker        : {len(census.ignored)}")
    by_crate: dict[str, int] = {}
    for path, count in census.counts.items():
        by_crate[_group(path)] = by_crate.get(_group(path), 0) + count
    for crate, count in sorted(by_crate.items()):
        print(f"    {crate}: {count}")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo-root", default=".", type=Path)
    parser.add_argument("--baseline-file", type=Path, default=DEFAULT_BASELINE)
    parser.add_argument("--exempt-file", type=Path, default=DEFAULT_EXEMPT)
    parser.add_argument("-v", "--verbose", action="store_true", help="list every counted and ignored occurrence")
    parser.add_argument(
        "--print-counts",
        action="store_true",
        help="print the current per-file counts in baseline format and exit 0 (does not gate)",
    )
    parser.add_argument(
        "--update-baseline",
        action="store_true",
        help="rewrite the baseline from the current counts; refuses if the total went up",
    )
    args = parser.parse_args(argv)
    repo_root = args.repo_root.resolve()

    try:
        exempt = load_exempt(args.exempt_file)
        census = take_census(repo_root, exempt)
    except (TokenizeError, OSError) as err:
        print(f"taffy-boundary-lint: cannot scan the sources: {err}", file=sys.stderr)
        return 2

    if args.print_counts:
        for path, count in sorted(census.counts.items()):
            print(f"{count} {path}")
        return 0

    try:
        baseline = load_baseline(args.baseline_file)
    except (OSError, ValueError) as err:
        print(f"taffy-boundary-lint: cannot read the baseline: {err}", file=sys.stderr)
        return 2

    if args.update_baseline:
        raised = sorted(path for path, count in census.counts.items() if count > baseline.get(path, 0))
        if raised:
            print("taffy-boundary-lint: refusing to raise the baseline; these files have more uses than pinned:")
            for path in raised:
                print(f"  {path}: {census.counts[path]} (pinned {baseline.get(path, 0)})")
            print("  Remove the new uses of Taffy output types instead, or mark a real exception with")
            print("  `// taffy-boundary-lint:ignore: <reason>`. If a pinned file moved, rename its line in")
            print("  the baseline by hand.")
            return 1
        old_total, new_total = sum(baseline.values()), sum(census.counts.values())
        args.baseline_file.write_text(format_baseline(census.counts, read_header(args.baseline_file)), encoding="utf-8")
        print(f"taffy-boundary-lint: baseline lowered ({old_total} -> {new_total}).")
        return 0

    if args.verbose:
        for path in sorted(census.occurrences):
            for occ in census.occurrences[path]:
                print(f"  counted: {occ}")
        for occ in census.ignored:
            print(f"  ignored: {occ}")

    verdict = compare(census, baseline)
    for path, line in census.unreasoned:
        print(f"note: {path}:{line}: `taffy-boundary-lint:ignore:` needs a reason after the colon; not honoured")

    if verdict.ok:
        print(f"taffy-boundary-lint: PASS ({sum(census.counts.values())} pinned occurrences, none new)")
        _print_summary(census, baseline)
        return 0

    print("taffy-boundary-lint: FAIL")
    for path in verdict.more:
        pinned = baseline.get(path, 0)
        found = census.counts[path]
        label = f"baseline {pinned}" if pinned else "no baseline entry"
        print(f"  {path}: {found} occurrence(s), {label} (+{found - pinned})")
        for occ in census.occurrences[path][:MAX_LISTED_PER_FILE]:
            print(f"    {occ.line}: {occ.text}  -> {_replacement(occ.name, path)}")
        if found > MAX_LISTED_PER_FILE:
            print(f"    ... {found - MAX_LISTED_PER_FILE} more (-v lists every occurrence)")
    if verdict.more:
        print("  A Taffy output type must not be named outside the adapter and the view crate.")
        print("  Use the owned name shown after `->`, or mark a real exception on the same line with")
        print("  `// taffy-boundary-lint:ignore: <reason>`.")
    for path in verdict.fewer:
        print(f"  {path}: {census.counts[path]} occurrence(s), baseline {baseline[path]} is stale")
    for path in verdict.gone:
        print(f"  {path}: baseline entry {baseline[path]} has no counted occurrences left")
    if verdict.fewer or verdict.gone:
        print("  Lower the baseline in the same change: scripts/taffy-boundary-lint.sh --update-baseline")
    _print_summary(census, baseline)
    return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
