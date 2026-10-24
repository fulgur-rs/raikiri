#!/usr/bin/env python3
"""scripts/lib/taffy_boundary_lint.py — Taffy boundary ratchet.

## Why this exists

raikiri-dom lays boxes out by calling Taffy, and Taffy's vocabulary used to leak
into every layer: the paint walk, the page pipeline, the inline engine, tables
and multicol all named `taffy::Layout`, `taffy::Size`, `taffy::LayoutInput`,
`taffy::Dimension`, `taffy::Style` and so on directly. The layout results, the
layout call types and the node handle are now types owned by this repository: a
dependency-free geometry crate (`raikiri-layout-geometry`) and a call-protocol
module inside raikiri-dom (`layout::protocol`). The style input, the lengths,
keywords and the style snapshot, is owned too (`raikiri-layout-style`). The only
code that may name Taffy's types is the adapter module (`layout::taffy_adapter`),
which converts between the owned names and Taffy's, and the projection module of
the view crate (`raikiri_layout_style::projection`), which implements Taffy's
style traits over computed values and keeps the compact form of style lengths.
This lint keeps the boundary from eroding: it counts the uses of Taffy's
vocabulary outside those places and fails on every one, so none can be added.

## Two tables of names

Output side (`OUTPUT_NAMES`): what a layout call returns or takes as its
protocol. That is geometry, layout results, the layout input and output, the
node handle, the cache, and the detailed grid reports that are handed straight
back to Taffy. Each one has an owned replacement or is adapter-only.

Input side (`INPUT_CLASSES`, flattened into `INPUT_NAMES`): the rest of Taffy's
public vocabulary that raikiri-dom could name, which raikiri builds or reads
before a layout call. That is the style traits, the length types, the keyword
enums and small value types, `Style`, the float and block formatting state, the
grid vocabulary, the tree traits, and the functions that run Taffy's algorithms
or build style values. The lengths, the keywords and small value types, `Style`
and the resolution helpers have an owned replacement, listed in
`OWNED_INPUT_NAMES`, and the failure output names it. The others (the style
traits, the flex and grid vocabulary, the float state, the tree traits and the
algorithm entry points) are the library's own interface and have none: the
projection and the adapter hold them, and the failure output says "no owned name
yet" (with the kind of name) instead of naming a type to move to.

The tables say which kind a name is and what it moves to; they do not decide
whether a use is counted. Any name a path reaches from the `taffy` crate is
counted. One that is in neither table is reported as an unclassified name (its
own line in the summary), so a name added by a Taffy upgrade cannot slip in
unpinned. Add it to a table to give it a kind.

## What is counted

An occurrence is a *path* that starts at the `taffy` crate and names an item of
it:

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

A path names a type when it has a capitalised segment after `taffy`: the first
one is the type, so `taffy::Size::ZERO` and `taffy::tree::LayoutInput` count as
`Size` and `LayoutInput`, while `taffy::GridPlacement::Line(1)` counts as
`GridPlacement` (an input-side enum), not as `Line`. A path with no capitalised
segment names a function (`taffy::compute_block_layout(..)`,
`use taffy::style_helpers::length`), and its last segment is the name. A path
that ends at a module is not a name by itself; importing a module is counted
separately, as above.

Comments and string literals are ignored, and only `.rs` files under `crates/`
are scanned (not `crates/*/target`). Names that arrive through an in-crate
alias are not seen at the use site: the alias definition is the counted
occurrence, and swapping it swaps every user. That includes the aliases that
rename the generic geometry types for style input, such as
`use taffy::Size as StyleSize;`.

`Size`, `Rect`, `Point` and `Line` are generic. Layout geometry uses them
(`Size<f32>`) and so does style input (`Size<Dimension>`,
`Rect<LengthPercentageAuto>`, `Point<Overflow>`, `Line<GridPlacement>`), and a
path cannot tell the two apart without type information. They are counted
regardless of the type argument, under their output-side names, so the number is
an upper bound. The hint in the failure output names the owned geometry type and
says that the style-input use has none yet.

A site that has to keep the Taffy name for a reason other than a missing
replacement is marked on the same line:

    use taffy::NodeId as TaffyNodeId; // taffy-boundary-lint:ignore: the tree traits take Taffy's id

The reason after the second colon is required; a marker without one is not
honoured. A marked occurrence is neither counted nor baselined; the summary
shows how many there are (`-v` lists them) so the exceptions stay visible. A
marker is never honoured on `Size`, `Rect`, `Point` or `Line` (counted whatever
their type argument, so a style-input alias cannot be hidden from the baseline)
or on a glob or module import (which stands for any number of names). The run
reports such a marker.

## Where it is not counted

`taffy_boundary_exempt.txt` lists path prefixes (a trailing `/` means a
directory) that may name Taffy types freely: the projection module of the view
crate, which implements Taffy's style traits over computed values, converts the
owned values to Taffy's, lists the grid tracks for Taffy and stores the lengths
in Taffy's compact form, and the adapter module, which converts between the
owned names and Taffy's and holds the tree-trait implementations and the calls
into Taffy's algorithms. The tests of those two modules live inside them.
Everything else is held to the ratchet, and the ratchet is at zero.

Test code outside the two modules that has to read the library-typed getters of
the projection, or call the library's tree traits on a session, marks its
imports with the ignore marker and says so in the reason. The marked
occurrences are summarised on every run, so the exceptions stay visible.

The owned names shown in the failure output come from two places. Geometry and
the box layout result live in the `raikiri-layout-geometry` crate and can be
used from any crate. The layout call protocol lives in raikiri-dom's
crate-private `layout::protocol` module (nothing outside raikiri-dom reads a
layout input or output), so a use of those names in another crate has no owned
name to move to and fails until the raikiri-dom API that replaces it exists.

Some output names have no owned replacement because they are handed straight
back to Taffy and belong in the adapter: `Cache`, `ClearState`,
`DetailedGridInfo`, `DetailedGridTracksInfo`, `DetailedGridItemsInfo` and
`DetailedLayoutInfo` (shown as "(adapter only)" in the failure output). A use
outside the adapter fails; it is not a reason to add an ignore marker.

Most input-side names that remain have no owned replacement: they are the
library's interface. The failure output shows the kind of name ("no owned name
yet (grid vocabulary)"). A change that needs a value of that kind should read it
through the style view or a purpose-specific accessor rather than name the Taffy
type outside the projection and the adapter. The resolution helper traits are
the exception: `MaybeMath` is owned by the geometry crate, and `MaybeResolve`
and `ResolveOrZero` are the view crate's traits, read through its `values`
module.

## The ratchet

`taffy_boundary_baseline.txt` pins the count per file. It is empty now, and a
unit test keeps it empty; the mechanism stays so that a migration of a new
vocabulary can be done the same way. A run fails when

  - a file has more occurrences than its baseline (or any, if it has no entry):
    new uses of a counted Taffy name;
  - a file has fewer occurrences than its baseline, or a baseline entry names a
    file that no longer exists or is now exempt: the pin is stale, and a stale
    pin would let later additions hide behind the slack.

Pinning per file instead of one total keeps parallel import-path changes from
conflicting on a single number, and names the file that regressed. A pin is a
count, not a list of names, so replacing one use with another in the same file
passes; the code change is what shows that.
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

# The output-side Taffy names this lint counts, with the owned name each one is
# moving to. The owned names live in the `raikiri-layout-geometry` crate
# (geometry and the box layout result) and in raikiri-dom's `layout::protocol`
# module (the call protocol). Names without an owned replacement are
# adapter-only: they are handed straight back to Taffy and stay in the adapter.
# `Size`, `Point`, `Line` and `Rect` are generic and also name style input; see
# GENERIC_GEOMETRY.
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
    "OofLayoutResult": "crate::layout::protocol::OofLayoutResult",
}

# Generic containers that name layout geometry (`Size<f32>`) and style input
# (`Size<Dimension>`) alike. They stay under their output-side names and are
# counted whatever the type argument is.
GENERIC_GEOMETRY = frozenset({"Size", "Point", "Line", "Rect"})

# The input-side Taffy names this lint counts, grouped by kind. The kind is shown
# in the failure output for the names that have no owned replacement yet. Type
# and trait names are capitalised; the lower-case entries are functions, matched
# by the last segment of a path that has no capitalised segment.
INPUT_CLASSES: dict[str, tuple[str, ...]] = {
    "style trait": (
        "CoreStyle",
        "BlockContainerStyle",
        "BlockItemStyle",
        "OofItemStyle",
        "FlexboxContainerStyle",
        "FlexboxItemStyle",
        "GridContainerStyle",
        "GridItemStyle",
        "CheapCloneStr",
    ),
    "length": (
        "Dimension",
        "LengthPercentage",
        "LengthPercentageAuto",
        "ExpandedDimension",
        "ExpandedLengthPercentage",
        "ExpandedLengthPercentageAuto",
        "CompactLength",
        "TaffyAuto",
        "TaffyZero",
        "TaffyMaxContent",
        "TaffyMinContent",
        "TaffyFitContent",
        "TaffyGridLine",
        "TaffyGridSpan",
        "FromLength",
        "FromPercent",
        "FromFr",
    ),
    "resolution helper": (
        "MaybeResolve",
        "ResolveOrZero",
        "MaybeMath",
    ),
    "style helper function": (
        "auto",
        "zero",
        "length",
        "percent",
        "fr",
        "flex",
        "fit_content",
        "min_content",
        "max_content",
        "minmax",
        "repeat",
        "line",
        "span",
        "evenly_sized_tracks",
    ),
    "keyword or small value": (
        "Direction",
        "Position",
        "Display",
        "Clear",
        "Float",
        "BoxSizing",
        "Overflow",
        "TextAlign",
        "Contain",
        "BoxGenerationMode",
        "ContainingBlockClaims",
        "FlexDirection",
        "FlexWrap",
        "AlignItems",
        "AlignSelf",
        "AlignContent",
        "JustifyContent",
        "JustifyItems",
        "JustifySelf",
        "AlignItemsKeyword",
        "AlignContentKeyword",
        "AlignmentSafety",
        "AbstractAxis",
        "MinMax",
    ),
    "Style struct": ("Style",),
    "float or block state": (
        "FloatContext",
        "FloatDirection",
        "FloatIntrinsicWidthCalculator",
        "BfcSlot",
        "ContentSlot",
        "PlacedFloatedBox",
        "BlockContext",
        "BlockFormattingContext",
    ),
    "grid vocabulary": (
        "TrackSizingFunction",
        "MinTrackSizingFunction",
        "MaxTrackSizingFunction",
        "ExpandedMinTrackSizingFunction",
        "ExpandedMaxTrackSizingFunction",
        "GridPlacement",
        "GenericGridPlacement",
        "GridTemplateComponent",
        "GenericGridTemplateComponent",
        "GridTemplateRepetition",
        "GenericRepetition",
        "RepetitionCount",
        "GridAutoFlow",
        "GridAutoTracks",
        "GridTemplateTracks",
        "GridTemplateArea",
        "GridTemplateAreas",
        "NamedGridLine",
        "TemplateLineNames",
    ),
    "tree trait": (
        "LayoutPartialTree",
        "LayoutBlockContainer",
        "LayoutFlexboxContainer",
        "LayoutGridContainer",
        "LayoutContainingBlock",
        "TraversePartialTree",
        "TraverseTree",
        "RoundTree",
        "PrintTree",
        "CacheTree",
        "TaffyTree",
        "TaffyError",
        "TaffyResult",
    ),
    "algorithm entry point": (
        "compute_block_align_content_offset",
        "compute_block_layout",
        "compute_cached_layout",
        "compute_flexbox_layout",
        "compute_grid_layout",
        "compute_hidden_layout",
        "compute_leaf_layout",
        "compute_oof_layout",
        "compute_oof_layout_for_area",
        "compute_root_layout",
        "resolve_static_offset",
    ),
}

# Name -> kind, for every input-side name.
INPUT_NAMES: dict[str, str] = {name: kind for kind, names in INPUT_CLASSES.items() for name in names}

# The input-side names that already have an owned replacement: the helper traits
# that resolve a length against a basis, the length types and their expanded
# forms, the keyword and small value types that style queries return, and the
# style struct of a box. The flex and grid vocabulary has none: the Taffy
# projection of the view crate is the one place that names it.
_OWNED_VALUE_NAMES = (
    "MaybeResolve",
    "ResolveOrZero",
    "Dimension",
    "LengthPercentage",
    "LengthPercentageAuto",
    "ExpandedDimension",
    "ExpandedLengthPercentage",
    "ExpandedLengthPercentageAuto",
    "Direction",
    "Position",
    "Display",
    "Clear",
    "Float",
    "BoxSizing",
    "Overflow",
    "Contain",
    "BoxGenerationMode",
    "ContainingBlockClaims",
    "AlignItems",
    "AlignSelf",
    "AlignContent",
    "AlignItemsKeyword",
    "AlignContentKeyword",
    "AlignmentSafety",
)
OWNED_INPUT_NAMES: dict[str, str] = {
    "MaybeMath": "raikiri_layout_geometry::MaybeMath",
    "Style": "raikiri_layout_style::BoxStyle (block flow and leaf sizing only, no flex or grid fields)",
    **{name: f"raikiri_layout_style::values::{name}" for name in _OWNED_VALUE_NAMES},
}

# A failing file lists at most this many occurrences; -v lists all of them.
MAX_LISTED_PER_FILE = 12

# Taffy's public modules. Importing one of them (or the crate itself) lets
# Taffy's names be written as `tree::NodeId`, which the path scan cannot see.
TAFFY_MODULES = frozenset(
    {"compute", "detailed_info", "geometry", "prelude", "style", "style_helpers", "traits", "tree", "util"}
)

# Pseudo names for the occurrences that are not a specific name.
GLOB_IMPORT = "*"
MODULE_IMPORT = "<module>"

# Occurrences an ignore marker is never honoured on: the generic geometry names
# (their style-input use must stay counted) and the imports that stand for any
# number of names.
MARKER_REFUSED = GENERIC_GEOMETRY | {GLOB_IMPORT, MODULE_IMPORT}

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
    refused_markers: tuple[int, ...] = ()


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


def _counted_name(segments: tuple[str, ...]) -> str | None:
    """The name a path starting at `taffy` refers to, if it names an item.

    Modules are lower case, so the first capitalised segment is the type (or
    the enum whose variant is meant). A path without one names a function, so
    its last segment is the name. A path that ends at a module is not a name.
    Whether the name is in a table only decides how a failure describes it.
    """
    for seg in segments[1:]:
        if seg[:1].isupper():
            return seg
    last = segments[-1]
    return None if last == "taffy" or last in TAFFY_MODULES else last


def scan_source(rel_path: str, src: str) -> FileScan:
    """Count the Taffy-name occurrences in one Rust source file."""
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
            name = _counted_name(leaf.segments)
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
        name = _counted_name(tuple(segs))
        if name is not None:
            line = inline.count("\n", 0, m.start()) + 1
            raw_hits.append((m.start(), line, name, "::".join(segs)))

    counted: list[Occurrence] = []
    ignored: list[Occurrence] = []
    unreasoned: list[int] = []
    refused: list[int] = []
    for _pos, line, name, text in sorted(raw_hits):
        raw_line = src_lines[line - 1] if line - 1 < len(src_lines) else ""
        occ = Occurrence(rel_path, line, name, text)
        has_marker = IGNORE_MARKER_RE.search(raw_line) is not None
        if has_marker and name not in MARKER_REFUSED:
            ignored.append(occ)
        else:
            if IGNORE_MARKER_WITHOUT_REASON_RE.search(raw_line):
                unreasoned.append(line)
            elif has_marker and line not in refused:
                refused.append(line)
            counted.append(occ)
    return FileScan(tuple(counted), tuple(ignored), tuple(unreasoned), tuple(refused))


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
    refused: list[tuple[str, int]]
    exempt_files: int


def take_census(repo_root: Path, exempt: list[str]) -> Census:
    census = Census({}, {}, [], [], [], 0)
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
        census.refused.extend((rel, line) for line in scan.refused_markers)
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
    if name in OWNED_INPUT_NAMES:
        owned = OWNED_INPUT_NAMES[name]
    elif name in INPUT_NAMES:
        return f"no owned name yet ({INPUT_NAMES[name]})"
    elif name not in OUTPUT_NAMES:
        return "no owned name yet (not in the name tables)"
    else:
        owned = OUTPUT_NAMES[name]
    if owned.startswith("crate::") and not path.startswith("crates/raikiri-dom/"):
        return "no owned name outside raikiri-dom yet"
    if name in GENERIC_GEOMETRY:
        return f"{owned} for layout geometry (none yet for style input)"
    return owned


def _side(name: str) -> str:
    """Which side a counted name is on: output, input, an import of many names, or in neither table."""
    if name in (GLOB_IMPORT, MODULE_IMPORT):
        return "import"
    if name in OUTPUT_NAMES:
        return "output"
    return "input" if name in INPUT_NAMES else "unclassified"


def _group(path: str) -> str:
    parts = path.split("/")
    return "/".join(parts[:2]) if len(parts) > 2 else path


def _print_summary(census: Census, baseline: dict[str, int]) -> None:
    total = sum(census.counts.values())
    print(f"  counted occurrences      : {total} in {len(census.counts)} file(s)")
    print(f"  pinned in baseline       : {sum(baseline.values())} in {len(baseline)} file(s)")
    print(f"  exempt files not scanned : {census.exempt_files}")
    print(f"  ignored by marker        : {len(census.ignored)}")
    by_side = {"output": 0, "input": 0, "unclassified": 0, "import": 0}
    for occs in census.occurrences.values():
        for occ in occs:
            by_side[_side(occ.name)] += 1
    print(f"  output-side names        : {by_side['output']}")
    print(f"  input-side names         : {by_side['input']}")
    print(f"  unclassified names       : {by_side['unclassified']}")
    print(f"  glob and module imports  : {by_side['import']}")
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
            print("  Remove the new uses of Taffy names instead, or mark a real exception with")
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
    for path, line in census.refused:
        print(f"note: {path}:{line}: `taffy-boundary-lint:ignore:` is not honoured on Size, Rect, Point, Line or a glob or module import; they are counted")

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
        print("  A Taffy name must not be written outside the adapter and the projection of the view crate.")
        print("  Use the owned name shown after `->`. Where it says there is no owned name yet, read the")
        print("  value through the style view or an existing helper instead of naming the Taffy type in a")
        print("  new place. A real exception is marked on the same line with")
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
