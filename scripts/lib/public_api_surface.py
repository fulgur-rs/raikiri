#!/usr/bin/env python3
"""scripts/lib/public_api_surface.py — the names a crate publishes, and who uses them.

## Why this exists

`public_api_lint.py` reads every item of the rustdoc JSON, hidden ones and
methods included, to find Taffy types. That is the right view for a leak check
and the wrong one for a person who asks "what does raikiri-html offer?" or
"which of it does fulgur call?". This module answers those two questions from
the same JSON.

## The surface

The surface of a crate is the set of names an outside user can write: the items
that the crate root reaches through its modules and its `pub use` lines, at the
level of types, functions, constants and modules. Methods, fields and trait
impls belong to their type and are not listed. A name is *hidden* when it, its
re-export or a module above it carries `#[doc(hidden)]`; hidden names are not
part of the promise a version number makes, and `cargo semver-checks` ignores
them too.

A glob re-export of a module of another crate (`pub use other::module::*`)
cannot be expanded from one crate's JSON. It is listed as `<path>::*` with the
module it names.

## Usage

    python3 public_api_surface.py --json-dir DIR CRATE...           # list the names
    python3 public_api_surface.py --json-dir DIR --usage SRC CRATE...
    python3 public_api_surface.py --json-dir DIR --exhaustive CRATE...

`--usage SRC` reads every `.rs` file under SRC and reports which names of the
crates it reaches through `use` declarations and inline paths. A path that goes
past a name (`raikiri_dom::Document::new`) counts as a use of the longest name
in it (`Document`): the report is at the level of names, so every method of a
used type is treated as used. A glob import is reported as such, because the
names it brings in cannot be known from the text.

`--exhaustive` lists the types declared in a crate that a user can match or
build completely, because they lack `#[non_exhaustive]`: the enums, and the
structs whose fields are all public. Adding a variant or a field to one of them
is a breaking change, which `docs/public-api.md` has a policy for.

`public_api_lint.py --surface`, `--usage` and `--exhaustive` call into this
module.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from collections.abc import Iterable, Iterator
from dataclasses import dataclass, field
from pathlib import Path

DECLARED_KINDS = frozenset(
    {
        "struct",
        "enum",
        "union",
        "trait",
        "function",
        "constant",
        "static",
        "type_alias",
        "macro",
        "proc_macro",
        "trait_alias",
        "extern_type",
    }
)


@dataclass(frozen=True)
class Name:
    """A name that an outside user can write for an item of the crate's API."""

    path: str
    kind: str
    hidden: bool
    # Where the item is defined: its canonical path, for a re-export of another
    # crate's item and for an item declared in a private module alike.
    canonical: str | None
    # The id of the item in this crate's JSON; None for a module, a glob and
    # an item of another crate.
    item_id: int | None = field(default=None, compare=False)


def item_kind(item: dict) -> str:
    return next(iter(item["inner"]))


# `hidden` as one option of a `#[doc(..)]` list, as in `#[doc(hidden)]` and
# `#[doc(alias = "x", hidden)]`; string literals are blanked first.
DOC_ATTRIBUTE_RE = re.compile(r"#\[doc\((.*)\)\]", re.DOTALL)
STRING_LITERAL_RE = re.compile(r'"(?:\\.|[^"\\])*"')
HIDDEN_OPTION_RE = re.compile(r"(?:^|,)\s*hidden\s*(?:,|$)")


def is_hidden(item: dict) -> bool:
    for attr in item.get("attrs", []):
        if not isinstance(attr, dict):
            continue
        found = DOC_ATTRIBUTE_RE.fullmatch(attr.get("other", "").strip())
        if found is not None and HIDDEN_OPTION_RE.search(STRING_LITERAL_RE.sub('""', found.group(1))):
            return True
    return False


def canonical_path(doc: dict, key: object) -> str | None:
    summary = doc["paths"].get(str(key))
    return "::".join(summary["path"]) if summary is not None else None


def names(doc: dict) -> list[Name]:
    """The names the root module of `doc` publishes, sorted by path.

    A path can name more than one thing: a module and a function of the same
    name (they live in different namespaces), a module and an enum variant, or
    globs of different crates under one prefix. Each is listed, so a name
    that a glob exports and an explicit item shadows stays in the list: the
    scan reads too much rather than too little.
    """
    index = doc["index"]
    root = index[str(doc["root"])]
    found: dict[tuple[str, int | None, str | None], Name] = {}

    def add(path: str, kind: str, hidden: bool, canonical: str | None, item_id: int | None = None) -> None:
        # The first way a name is reached wins; a visible one beats a hidden one.
        slot = (path, item_id, canonical)
        known = found.get(slot)
        if known is None or (known.hidden and not hidden):
            found[slot] = Name(path, kind, hidden, canonical, item_id)

    def module(item: dict, prefix: str, hidden: bool, active: tuple[int, ...]) -> None:
        key = item["id"]
        if key in active:
            return
        for child in item["inner"]["module"]["items"]:
            entry = index.get(str(child))
            if entry is not None:
                declared(entry, prefix, hidden, (*active, key))

    def declared(entry: dict, prefix: str, hidden: bool, active: tuple[int, ...]) -> None:
        kind = item_kind(entry)
        hidden = hidden or is_hidden(entry)
        if kind == "use":
            reexport(entry, prefix, hidden, active)
        elif kind == "module":
            path = f"{prefix}::{entry['name']}"
            add(path, "module", hidden, None)
            module(entry, path, hidden, active)
        elif kind == "extern_crate":
            # `pub extern crate dep;` publishes the whole crate under this name.
            # rustdoc puts the name it is published under in `name` and, for
            # `extern crate dep as other;`, the crate itself in `rename`.
            if entry.get("visibility") == "public":
                body = entry["inner"]["extern_crate"]
                add(f"{prefix}::{body['name']}", kind, hidden, body.get("rename") or body["name"])
        elif kind in DECLARED_KINDS:
            add(f"{prefix}::{entry['name']}", kind, hidden, canonical_path(doc, entry["id"]), entry["id"])

    def reexport(entry: dict, prefix: str, hidden: bool, active: tuple[int, ...]) -> None:
        body = entry["inner"]["use"]
        target_id = body.get("id")
        target = index.get(str(target_id)) if isinstance(target_id, int) else None
        if target is not None and target.get("crate_id") != 0:
            target = None
        if body["is_glob"]:
            if target is not None and item_kind(target) == "module":
                module(target, prefix, hidden or is_hidden(target), active)
            elif target is not None and item_kind(target) == "enum":
                # `pub use path::E::*;` publishes the variants at the use site.
                hidden = hidden or is_hidden(target)
                for variant_id in target["inner"]["enum"]["variants"]:
                    variant = index.get(str(variant_id))
                    if variant is not None:
                        add(
                            f"{prefix}::{variant['name']}",
                            "variant",
                            hidden or is_hidden(variant),
                            canonical_path(doc, variant_id),
                            variant_id,
                        )
            else:
                origin = canonical_path(doc, target_id) or body.get("source")
                add(f"{prefix}::*", "glob", hidden, origin)
            return
        path = f"{prefix}::{body['name']}"
        if target is None:
            summary = doc["paths"].get(str(target_id))
            kind = summary["kind"] if summary is not None else "re-export"
            add(path, kind, hidden, canonical_path(doc, target_id) or body.get("source"))
        elif item_kind(target) == "module":
            hidden = hidden or is_hidden(target)
            add(path, "module", hidden, None)
            module(target, path, hidden, active)
        else:
            add(path, item_kind(target), hidden or is_hidden(target), canonical_path(doc, target_id), target_id)

    module(root, root["name"], False, ())
    return sorted(found.values(), key=lambda name: (name.path, name.kind, name.canonical or ""))


def members(item: dict) -> list[int]:
    """The ids of the items that belong to `item`: fields, variants, impls and their items.

    `child_ids` in public_api_lint.py reads the same JSON for the Taffy walk and
    does not follow the fields of a struct-like variant; keep the two in step
    when the rustdoc format changes.
    """
    kind = item_kind(item)
    body = item["inner"][kind]
    if not isinstance(body, dict):
        return []
    found: list[int] = []
    # `implementations` is how a trait lists the impls of it, wherever they are written.
    for key in ("fields", "variants", "impls", "items", "implementations"):
        value = body.get(key)
        if isinstance(value, list):
            found.extend(entry for entry in value if isinstance(entry, int))
    shape = body.get("kind")
    if isinstance(shape, dict):
        for variant in ("plain", "struct"):
            fields = shape.get(variant, {}).get("fields") if isinstance(shape.get(variant), dict) else None
            if isinstance(fields, list):
                found.extend(entry for entry in fields if isinstance(entry, int))
        if isinstance(shape.get("tuple"), list):
            found.extend(entry for entry in shape["tuple"] if isinstance(entry, int))
    if kind == "module":
        return []  # modules are walked through their names
    return found


def referenced_ids(node: object) -> Iterator[int]:
    """Every id that a piece of an item refers to."""
    if isinstance(node, dict):
        target = node.get("id")
        if isinstance(target, int):
            yield target
        for child in node.values():
            yield from referenced_ids(child)
    elif isinstance(node, list):
        for child in node:
            yield from referenced_ids(child)


def public_roots(doc: dict) -> list[int]:
    """The ids of the items of the crate that are `pub` and that no `#[doc(hidden)]` covers.

    rustdoc keeps in the JSON the items that the API reaches, so this is a
    second way to the visible items that does not depend on how the names are
    worked out: a name the surface reads wrongly cannot hide an item. An item
    is covered when it is hidden itself, belongs to a hidden type or module (a
    re-export in a hidden module included), or is what a hidden `pub use`
    points at, or when only hidden names lead to it.
    """
    index = doc["index"]
    covered: set[int] = set()
    stack = [int(key) for key, item in index.items() if is_hidden(item)]
    stack.extend(name.item_id for name in names(doc) if name.hidden and name.item_id is not None)
    while stack:
        key = stack.pop()
        if key in covered:
            continue
        covered.add(key)
        item = index.get(str(key))
        if item is None:
            continue
        kind = item_kind(item)
        if kind == "module":
            stack.extend(item["inner"]["module"]["items"])
        elif kind == "use":
            target = item["inner"]["use"].get("id")
            if isinstance(target, int):
                stack.append(target)
        else:
            stack.extend(members(item))
    return [
        int(key)
        for key, item in index.items()
        if int(key) not in covered and item.get("crate_id") == 0 and item.get("visibility") == "public"
    ]


def visible_items(doc: dict) -> Iterator[tuple[int, dict]]:
    """Every item of the crate that the visible names reach, with the members of its types.

    That is the items the names stand for and the other `pub` items that are
    not hidden (`public_roots`), their fields, variants and impls (a trait's
    impls included), and the types of the crate that their signatures mention,
    nameable or not: a public function that returns a type of a private module
    hands its fields and methods to the caller.

    Blanket and synthetic impls are skipped: they come from the traits that
    other crates implement for every type, not from the API of this crate.
    """
    index = doc["index"]
    seen: set[int] = set()
    stack = [name.item_id for name in names(doc) if not name.hidden and name.item_id is not None]
    stack.extend(public_roots(doc))
    while stack:
        key = stack.pop()
        if key in seen:
            continue
        seen.add(key)
        item = index.get(str(key))
        if item is None or item.get("crate_id") != 0 or is_hidden(item):
            continue
        kind = item_kind(item)
        if kind == "impl" and (item["inner"]["impl"].get("blanket_impl") or item["inner"]["impl"].get("is_synthetic")):
            continue
        yield key, item
        stack.extend(members(item))
        stack.extend(referenced_ids(item["inner"]))


def format_names(listed: Iterable[Name], *, with_hidden: bool = False) -> str:
    lines = []
    for name in listed:
        if name.hidden and not with_hidden:
            continue
        flag = " (hidden)" if name.hidden else ""
        origin = f"  <- {name.canonical}" if name.canonical and name.canonical != name.path else ""
        lines.append(f"{name.kind:10s} {name.path}{flag}{origin}")
    return "\n".join(lines) + ("\n" if lines else "")


def has_attribute(item: dict, name: str) -> bool:
    """Whether `item` carries `#[name]`. rustdoc stores some attributes as a bare string, others as `other`."""
    for attr in item.get("attrs", []):
        if attr == name:
            return True
        if isinstance(attr, dict) and attr.get("other", "").startswith(f"#[{name}"):
            return True
    return False


def exhaustive_types(doc: dict) -> list[tuple[Name, str]]:
    """The visible types a user can match or build completely, with what that exposes.

    An enum without `#[non_exhaustive]` can be matched without a wildcard arm, so
    a new variant breaks that match. A struct without it whose fields are all
    public can be built with a struct literal and destructured without `..`, so a
    new field breaks both. A struct with a private field can do neither outside
    its crate and is left out, and so is an item of another crate (list that
    crate itself).
    """
    index = doc["index"]
    found = []
    seen: set[int] = set()
    # A type reachable under two names (at the root and through its module) is
    # listed once, under the shorter path.
    for name in sorted(names(doc), key=lambda n: (n.path.count("::"), n.path)):
        if name.hidden or name.item_id is None or name.kind not in ("struct", "enum") or name.item_id in seen:
            continue
        seen.add(name.item_id)
        item = index[str(name.item_id)]
        if has_attribute(item, "non_exhaustive"):
            continue
        if name.kind == "enum":
            found.append((name, f"{len(item['inner']['enum']['variants'])} variants"))
            continue
        shape = item["inner"]["struct"]["kind"]
        if isinstance(shape, dict) and "plain" in shape:
            fields, stripped = shape["plain"]["fields"], shape["plain"]["has_stripped_fields"]
        elif isinstance(shape, dict) and "tuple" in shape:
            fields, stripped = [f for f in shape["tuple"] if f is not None], None in shape["tuple"]
        else:
            continue
        if fields and not stripped:
            found.append((name, f"{len(fields)} public fields"))
    return sorted(found, key=lambda entry: entry[0].path)


def format_exhaustive(listed: Iterable[tuple[Name, str]]) -> str:
    lines = [f"{name.kind:10s} {name.path}  ({detail})" for name, detail in listed]
    return "\n".join(lines) + ("\n" if lines else "")


# ---------------------------------------------------------------------------
# Usage: which names does a body of Rust source reach?
# ---------------------------------------------------------------------------


def blank_comments_and_strings(source: str) -> str:
    """`source` with comments and the inside of string and char literals blanked.

    Newlines stay, so that offsets keep their line. A lifetime (`'a`) is not a
    char literal: a quote starts one only when it closes within two characters.
    """
    out: list[str] = []
    i, n = 0, len(source)

    def blank(text: str) -> str:
        return "".join(c if c == "\n" else " " for c in text)

    while i < n:
        two = source[i : i + 2]
        c = source[i]
        if two == "//":
            j = source.find("\n", i)
            j = n if j < 0 else j
            out.append(blank(source[i:j]))
            i = j
        elif two == "/*":
            depth, j = 1, i + 2
            while j < n and depth:
                if source.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif source.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            out.append(blank(source[i:j]))
            i = j
        elif c in "rbc" and (raw := RAW_STRING_RE.match(source, i)) and (i == 0 or not (source[i - 1].isalnum() or source[i - 1] == "_")):
            hashes = len(raw.group(1))
            end = source.find('"' + "#" * hashes, raw.end())
            j = n if end < 0 else end + 1 + hashes
            out.append(blank(source[i:j]))
            i = j
        elif c == '"':
            j = i + 1
            while j < n and source[j] != '"':
                j += 2 if source[j] == "\\" else 1
            if j >= n:  # no closing quote: the rest of the file is the literal
                out.append(blank(source[i:]))
                i = n
                continue
            out.append('"' + blank(source[i + 1 : j]) + '"')
            i = j + 1
        elif c == "'" and (source[i + 1 : i + 2] == "\\" or source[i + 2 : i + 3] == "'"):
            j = source.find("'", i + 2 if source[i + 1 : i + 2] != "\\" else i + 3)
            j = n if j < 0 else j + 1
            out.append("'" + blank(source[i + 1 : j - 1]) + "'")
            i = j
        else:
            out.append(c)
            i += 1
    return "".join(out)


# A raw string, with the `b` or `c` prefix of a raw byte or C string if it has one.
RAW_STRING_RE = re.compile(r'[bc]?r(#*)"')
# `use raikiri_x as alias;` and `extern crate raikiri_x as alias;`
ALIAS_RE = re.compile(r"\b(?:use|extern\s+crate)\s+(?:::)?(raikiri_[a-z0-9_]+)\s+as\s+([A-Za-z_][A-Za-z0-9_]*)\s*;")
TOKEN_RE = re.compile(r"::|\{|\}|,|\*|[A-Za-z_][A-Za-z0-9_]*")
USE_RE = re.compile(r"\buse\s+(?:::)?(?=raikiri_)([^;]*);")
INLINE_RE = re.compile(r"(?<![A-Za-z0-9_])(raikiri_[a-z0-9_]+)((?:::[A-Za-z_][A-Za-z0-9_]*)*)")


def expand_use_tree(text: str) -> list[str]:
    """The paths a use tree names: `a::{b, c::d as e, *}` is `a::b`, `a::c::d`, `a::*`."""
    tokens = TOKEN_RE.findall(text)
    position = 0
    result: list[str] = []

    def tree(prefix: list[str]) -> None:
        nonlocal position
        path = list(prefix)
        while position < len(tokens):
            token = tokens[position]
            if token == "{":
                position += 1
                while position < len(tokens) and tokens[position] != "}":
                    tree(path)
                    if position < len(tokens) and tokens[position] == ",":
                        position += 1
                position += 1  # the closing brace
                return
            if token == "*":
                position += 1
                result.append("::".join([*path, "*"]))
                return
            if token == "::":
                position += 1
                continue
            if token in (",", "}"):
                break
            if token == "as":
                position += 2  # `as` and the alias
                break
            position += 1
            if token != "self":
                path.append(token)
        if path:
            result.append("::".join(path))

    tree([])
    return result


def used_paths(source: str) -> tuple[set[str], set[str]]:
    """(paths, globs) that `source` reaches into `raikiri_*` crates."""
    cleaned = blank_comments_and_strings(source)
    for crate, alias in ALIAS_RE.findall(cleaned):
        # A path through the alias is a path through the crate.
        cleaned = re.sub(rf"(?<![A-Za-z0-9_:]){alias}::", f"{crate}::", cleaned)
    paths: set[str] = set()
    globs: set[str] = set()
    for match in USE_RE.finditer(cleaned):
        for path in expand_use_tree(match.group(1)):
            (globs if path.endswith("::*") else paths).add(path)
    # The use declarations are read; look for inline paths in the rest.
    rest = USE_RE.sub(lambda match: re.sub(r"[^\n]", " ", match.group(0)), cleaned)
    for match in INLINE_RE.finditer(rest):
        if match.group(2):
            paths.add(match.group(1) + match.group(2))
    return paths, globs


@dataclass
class Usage:
    used: dict[str, Name] = field(default_factory=dict)
    hidden: dict[str, Name] = field(default_factory=dict)
    unknown: set[str] = field(default_factory=set)
    globs: set[str] = field(default_factory=set)
    files: int = 0


def resolve(path: str, surface: dict[str, Name]) -> Name | None:
    """The longest name of `surface` that `path` starts with, or None."""
    segments = path.split("::")
    for end in range(len(segments), 1, -1):
        found = surface.get("::".join(segments[:end]))
        if found is not None:
            return found
    return None


def rust_files(root: Path) -> Iterator[Path]:
    skipped = {"target", ".git", "node_modules"}
    for path in sorted(root.rglob("*.rs")):
        if not skipped.intersection(path.relative_to(root).parts):
            yield path


def scan_usage(root: Path, surfaces: dict[str, dict[str, Name]]) -> Usage:
    """The names of `surfaces` (by crate identifier) that the sources under `root` reach."""
    usage = Usage()
    for file in rust_files(root):
        usage.files += 1
        try:
            source = file.read_text(encoding="utf-8")
        except (OSError, UnicodeDecodeError):
            continue
        paths, globs = used_paths(source)
        for path in paths:
            crate = path.split("::", 1)[0]
            if crate not in surfaces or crate == path:  # a bare crate name names no item
                continue
            name = resolve(path, surfaces[crate])
            if name is None:
                usage.unknown.add(path)
            elif name.hidden:
                usage.hidden[name.path] = name
            else:
                usage.used[name.path] = name
        usage.globs.update(glob for glob in globs if glob.split("::", 1)[0] in surfaces)
    return usage


def format_usage(usage: Usage, crates: Iterable[str]) -> str:
    lines = [f"read {usage.files} Rust files"]
    for crate in crates:
        used = sorted((name for name in usage.used.values() if name.path.split("::", 1)[0] == crate), key=lambda n: n.path)
        lines.append(f"{crate}: {len(used)} names used")
        lines.extend(f"  {name.kind:10s} {name.path}" for name in used)
    for title, entries in (
        ("hidden names used (outside the promise of a version number)", sorted(usage.hidden)),
        ("paths that match no name of the crate (renamed, removed or private?)", sorted(usage.unknown)),
        ("glob imports (the names they bring in are not known)", sorted(usage.globs)),
    ):
        if entries:
            lines.append(f"{title}: {len(entries)}")
            lines.extend(f"  {entry}" for entry in entries)
    return "\n".join(lines) + "\n"


def load_names(json_dir: Path, crate: str) -> list[Name]:
    path = json_dir / (crate.replace("-", "_") + ".json")
    with path.open(encoding="utf-8") as handle:
        return names(json.load(handle))


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("crates", nargs="+", help="crate names, e.g. raikiri-html")
    parser.add_argument("--json-dir", required=True, type=Path)
    parser.add_argument("--usage", type=Path, metavar="SRC")
    parser.add_argument("--with-hidden", action="store_true")
    parser.add_argument("--exhaustive", action="store_true")
    args = parser.parse_args(argv)
    if args.exhaustive:
        for crate in args.crates:
            path = args.json_dir / (crate.replace("-", "_") + ".json")
            with path.open(encoding="utf-8") as handle:
                sys.stdout.write(format_exhaustive(exhaustive_types(json.load(handle))))
        return 0
    listed = {crate: load_names(args.json_dir, crate) for crate in args.crates}
    if args.usage is None:
        for crate, items in listed.items():
            sys.stdout.write(format_names(items, with_hidden=args.with_hidden))
        return 0
    surfaces = {crate.replace("-", "_"): {n.path: n for n in items} for crate, items in listed.items()}
    sys.stdout.write(format_usage(scan_usage(args.usage, surfaces), surfaces))
    return 0


if __name__ == "__main__":
    sys.exit(main())
