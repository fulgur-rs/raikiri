#!/usr/bin/env python3
"""scripts/lib/public_api_lint.py — no Taffy type in the public API.

## Why this exists

Taffy computes the layout, but it is an implementation detail of raikiri-dom
and of the style view crate. The crates that other code builds on (raikiri-dom,
raikiri-html, raikiri-paint, raikiri-traits, raikiri) must not make a consumer
name a Taffy type: a signature, a field, a trait bound or a trait impl that
mentions one ties every consumer to Taffy's version. The import-path lint
(`taffy_boundary_lint.py`) looks at source text; this one looks at what the
compiler publishes, so it also sees what a text scan cannot: an item that is
public only through a re-export, a `#[doc(hidden)]` item that other crates of
the workspace still call, and a trait implemented for a public type.

## How it looks

It asks rustdoc for the public API of every crate in `CHECKED_CRATES` as JSON
(`--output-format json`, with `--document-hidden-items` so that hidden items
count) and walks every item the crate publishes. An item is reported when
anything inside it (a parameter, a return type, a field type, a generic bound,
the trait or the self type of an impl, a re-exported path) resolves to an item
of the `taffy` crate.

`cargo doc --workspace` produces all the JSON in one run, with the dependency
build that the rustdoc step of the gate has already done: the run takes about
ten seconds on a warm target directory. The rustdoc JSON output is unstable, so
rustdoc, and only rustdoc, is started through a one-line wrapper that sets
`RUSTC_BOOTSTRAP=1` (the `RUSTDOC` environment variable), which keeps the
compiler's own invocations, and the artifacts of the dependencies, as they are.
The format changes with the toolchain; `KNOWN_FORMAT_VERSIONS` names the ones
this walker was written against, and another one stops the run (exit 2) until
the walker has been checked against it.

## The view crate

`raikiri-layout-style` (`VIEW_CRATES`) implements Taffy's style traits over
computed values; it names Taffy types on purpose, and nothing is reported for
it. It is scanned all the same: a view crate whose scan finds no Taffy item
means the walker cannot see them, which stops the run (exit 2), so a change in
the JSON format cannot turn the check into a silent pass.

## Usage

    python3 public_api_lint.py --repo-root PATH [-v]
    python3 public_api_lint.py --repo-root PATH --dump CRATE

`--dump CRATE` prints the public API of one crate, one line per item with every
id replaced by the path it names, which is the same text for two builds of the
same API. Dump a base checkout and a head checkout and `diff` them to check
that a change left a crate's public signatures alone (raikiri-html is the
surface fulgur depends on).

`--json-dir DIR` reads the JSON files from DIR (named `<crate with _>.json`)
instead of running cargo.

Exit status: 0 on pass, 1 when an item names a Taffy type, 2 on a tooling error
(cargo failed, a JSON file is missing or of an unknown format, the walker went
blind).
"""

from __future__ import annotations

import argparse
import json
import os
import stat
import subprocess
import sys
import tempfile
from collections.abc import Iterable, Iterator
from dataclasses import dataclass
from pathlib import Path

# Crates whose public API may not name a Taffy type.
CHECKED_CRATES = (
    "raikiri-dom",
    "raikiri-html",
    "raikiri-paint",
    "raikiri-traits",
    "raikiri",
    "raikiri-layout-geometry",
)
# Crates that name Taffy types by design; scanned only to prove the walker sees.
VIEW_CRATES = ("raikiri-layout-style",)
# The crate that must not appear in the signature of a checked item.
FORBIDDEN_CRATE = "taffy"
# `format_version` of the rustdoc JSON of the pinned toolchain.
KNOWN_FORMAT_VERSIONS = frozenset({56})

RUSTDOC_FLAGS = "-Z unstable-options --output-format json --document-hidden-items --cap-lints allow"
# Keys that hold the ids of the items inside an item.
CHILD_KEYS = ("items", "fields", "variants", "impls")


class ToolError(Exception):
    """The run could not be completed; not a finding about the API."""


@dataclass(frozen=True)
class Leak:
    """A public item that names Taffy."""

    crate: str
    path: str
    kind: str
    names: tuple[str, ...]


def json_file_name(crate: str) -> str:
    return crate.replace("-", "_") + ".json"


def load_doc(path: Path) -> dict:
    """Read one rustdoc JSON file and refuse the ones this walker cannot read."""
    try:
        with path.open(encoding="utf-8") as handle:
            doc = json.load(handle)
    except (OSError, ValueError) as error:
        raise ToolError(f"cannot read {path}: {error}") from error
    version = doc.get("format_version")
    if version not in KNOWN_FORMAT_VERSIONS:
        raise ToolError(
            f"{path}: rustdoc JSON format_version {version} is not one of "
            f"{sorted(KNOWN_FORMAT_VERSIONS)}; check the walker in "
            "scripts/lib/public_api_lint.py against the new format, then add it"
        )
    if doc.get("includes_private"):
        raise ToolError(f"{path}: the JSON includes private items; its items are not the public API")
    return doc


def local_items(doc: dict) -> Iterator[tuple[int, dict]]:
    """The items of the crate itself (crate id 0) that the JSON publishes."""
    for key, item in doc["index"].items():
        if item.get("crate_id") == 0:
            yield int(key), item


def taffy_crate_ids(doc: dict) -> frozenset[int]:
    return frozenset(
        int(key) for key, crate in doc["external_crates"].items() if crate["name"] == FORBIDDEN_CRATE
    )


def taffy_names_in(doc: dict, node: object, taffy_ids: frozenset[int]) -> set[str]:
    """The paths of the Taffy items that `node` (a piece of an item) refers to.

    Every reference to an item in rustdoc JSON is a mapping with an integer
    `id`; `paths` says which crate the item belongs to and where.
    """
    found: set[str] = set()
    paths = doc["paths"]

    def walk(value: object) -> None:
        if isinstance(value, dict):
            target = value.get("id")
            if isinstance(target, int):
                summary = paths.get(str(target))
                if summary is not None and summary["crate_id"] in taffy_ids:
                    found.add("::".join(summary["path"]))
            for child in value.values():
                walk(child)
        elif isinstance(value, list):
            for child in value:
                walk(child)

    walk(node)
    return found


def item_kind(item: dict) -> str:
    return next(iter(item["inner"]))


def int_entries(value: object) -> list[int]:
    return [entry for entry in value if isinstance(entry, int)] if isinstance(value, list) else []


def child_ids(inner: dict) -> list[int]:
    """The ids of the items an item contains (members, fields, variants, impls)."""
    body = next(iter(inner.values()))
    if not isinstance(body, dict):
        return []
    found = [entry for key in CHILD_KEYS for entry in int_entries(body.get(key))]
    kind = body.get("kind")
    if isinstance(kind, dict):
        plain = kind.get("plain")
        if isinstance(plain, dict):
            found.extend(int_entries(plain.get("fields")))
        found.extend(int_entries(kind.get("tuple")))
    return found


class Names:
    """Readable paths for the items of one crate's JSON."""

    def __init__(self, doc: dict) -> None:
        self.doc = doc
        self.parent: dict[int, int] = {}
        for key, item in local_items(doc):
            for child in child_ids(item["inner"]):
                self.parent.setdefault(child, key)
        self.cache: dict[int, str] = {}

    def path_of(self, key: int) -> str:
        if key in self.cache:
            return self.cache[key]
        self.cache[key] = f"#{key}"  # a cycle ends here
        item = self.doc["index"].get(str(key))
        summary = self.doc["paths"].get(str(key))
        if summary is not None:
            path = "::".join(summary["path"])
        elif item is None:
            path = f"#{key}"
        else:
            own = self.own_name(item)
            parent = self.parent.get(key)
            path = f"{self.path_of(parent)}::{own}" if parent is not None else own
        self.cache[key] = path
        return path

    def own_name(self, item: dict) -> str:
        if item.get("name") is not None:
            return item["name"]
        if item_kind(item) == "impl":
            impl = item["inner"]["impl"]
            subject = self.type_name(impl["for"])
            if impl.get("trait") is None:
                return f"<impl {subject}>"
            return f"<impl {self.type_name({'resolved_path': impl['trait']})} for {subject}>"
        return f"<{item_kind(item)}>"

    def type_name(self, node: object) -> str:
        """`node`, a type, in the way Rust writes it, as far as a path needs."""
        if not isinstance(node, dict) or not node:
            return "?"
        kind, body = next(iter(node.items()))
        if kind == "resolved_path":
            text = self.path_of(body["id"]) if isinstance(body.get("id"), int) else body["path"]
            args = body.get("args")
            listed = args.get("angle_bracketed", {}).get("args", []) if isinstance(args, dict) else []
            shown = [self.type_name(arg["type"]) for arg in listed if isinstance(arg, dict) and "type" in arg]
            return f"{text}<{', '.join(shown)}>" if shown else text
        if kind in ("generic", "primitive"):
            return str(body)
        if kind == "borrowed_ref":
            return "&" + self.type_name(body["type"])
        if kind == "tuple":
            return "(" + ", ".join(self.type_name(entry) for entry in body) + ")"
        if kind == "slice":
            return "[" + self.type_name(body) + "]"
        if kind == "array":
            return "[" + self.type_name(body["type"]) + "; _]"
        return kind


def canonical(names: Names, node: object) -> str:
    """`node` as one line of JSON in which every id is the path it names.

    Ids are numbered per build, so the text of the same API from two builds is
    only the same once they are gone. The ids of the contained items are
    dropped: those items have lines of their own.
    """

    def resolve(value: object) -> object:
        if isinstance(value, dict):
            out = {}
            for key, child in value.items():
                if key in CHILD_KEYS or key == "implementations":
                    continue
                if key == "tuple" and isinstance(child, list) and all(
                    entry is None or isinstance(entry, int) for entry in child
                ):
                    continue  # the fields of a tuple struct or variant, not a tuple type
                if key == "id" and isinstance(child, int):
                    out[key] = names.path_of(child)
                else:
                    out[key] = resolve(child)
            return out
        if isinstance(value, list):
            return [resolve(child) for child in value]
        return value

    return json.dumps(resolve(node), sort_keys=True, separators=(",", ":"))


def find_leaks(crate: str, doc: dict) -> list[Leak]:
    """The items of `crate` that name a Taffy type."""
    taffy_ids = taffy_crate_ids(doc)
    if not taffy_ids:
        return []
    names = Names(doc)
    leaks = []
    for key, item in local_items(doc):
        found = taffy_names_in(doc, item["inner"], taffy_ids)
        if found:
            leaks.append(Leak(crate, names.path_of(key), item_kind(item), tuple(sorted(found))))
    return sorted(leaks, key=lambda leak: (leak.path, leak.kind, leak.names))


def api_lines(doc: dict) -> list[str]:
    """One line per public item: its path, its kind and its canonical signature."""
    names = Names(doc)
    lines = []
    for key, item in local_items(doc):
        hidden = any(
            isinstance(attr, dict) and "doc(hidden)" in attr.get("other", "")
            for attr in item.get("attrs", [])
        )
        flags = " hidden" if hidden else ""
        lines.append(
            f"{names.path_of(key)} [{item_kind(item)}{flags}] {canonical(names, item['inner'])}"
        )
    return sorted(lines)


def cargo_target_directory(repo_root: Path) -> Path:
    try:
        output = subprocess.run(
            ["cargo", "metadata", "--no-deps", "--format-version", "1", "--locked"],
            cwd=repo_root,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
        return Path(json.loads(output)["target_directory"])
    except (OSError, subprocess.CalledProcessError, ValueError, KeyError) as error:
        raise ToolError(f"cargo metadata failed: {error}") from error


def generate_json(repo_root: Path, crates: Iterable[str]) -> Path:
    """Run `cargo doc` for rustdoc JSON; return the directory it wrote it to."""
    out_dir = cargo_target_directory(repo_root) / "doc"
    for crate in crates:
        (out_dir / json_file_name(crate)).unlink(missing_ok=True)
    with tempfile.TemporaryDirectory(prefix="public-api-lint-") as scratch:
        wrapper = Path(scratch) / "rustdoc"
        wrapper.write_text('#!/bin/sh\nRUSTC_BOOTSTRAP=1 exec rustdoc "$@"\n', encoding="utf-8")
        wrapper.chmod(wrapper.stat().st_mode | stat.S_IXUSR)
        env = dict(os.environ, RUSTDOC=str(wrapper), RUSTDOCFLAGS=RUSTDOC_FLAGS)
        result = subprocess.run(
            ["cargo", "doc", "--no-deps", "--workspace", "--locked"],
            cwd=repo_root,
            env=env,
            capture_output=True,
            text=True,
            check=False,
        )
    if result.returncode != 0:
        sys.stderr.write(result.stdout + result.stderr)
        raise ToolError("cargo doc failed while writing the rustdoc JSON")
    return out_dir


def run_check(json_dir: Path, verbose: bool) -> int:
    leaks: list[Leak] = []
    counts: dict[str, tuple[int, int]] = {}
    for crate in CHECKED_CRATES:
        doc = load_doc(json_dir / json_file_name(crate))
        published = sum(1 for _ in local_items(doc))
        if published == 0:
            raise ToolError(f"{crate}: the JSON lists no public item; the walker cannot see the API")
        found = find_leaks(crate, doc)
        leaks.extend(found)
        counts[crate] = (published, len(found))
    view_counts: dict[str, int] = {}
    for crate in VIEW_CRATES:
        doc = load_doc(json_dir / json_file_name(crate))
        view_counts[crate] = len(find_leaks(crate, doc))
        if view_counts[crate] == 0:
            raise ToolError(
                f"{crate} is the view crate and names Taffy types, yet the walker found none; "
                "the rustdoc JSON format has probably changed in a way it cannot read"
            )

    if leaks:
        print("public-api-lint: FAIL")
        print("  these public items name a Taffy type; consumers would have to depend on Taffy:")
        for leak in leaks:
            print(f"  {leak.crate}: {leak.path} ({leak.kind}): {', '.join(leak.names)}")
        print("  Use an owned type (raikiri-layout-geometry, or raikiri-dom's own), or make the item private.")
        return 1

    print("public-api-lint: PASS (no public item of the checked crates names a Taffy type)")
    if verbose:
        for crate, (published, _) in counts.items():
            print(f"  {crate}: {published} public items, none names Taffy")
        for crate, count in view_counts.items():
            print(f"  {crate}: {count} items name Taffy (view crate, allowed)")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--repo-root", required=True, type=Path)
    parser.add_argument("--json-dir", type=Path, help="read the rustdoc JSON from here instead of running cargo")
    parser.add_argument("--dump", metavar="CRATE", help="print the public API of one crate instead of checking")
    parser.add_argument("-v", "--verbose", action="store_true")
    args = parser.parse_args(argv)
    try:
        crates = (*CHECKED_CRATES, *VIEW_CRATES)
        if args.dump is not None and args.dump not in crates:
            raise ToolError(f"--dump takes one of: {', '.join(crates)}")
        json_dir = args.json_dir or generate_json(args.repo_root, crates)
        if args.dump is not None:
            sys.stdout.write("\n".join(api_lines(load_doc(json_dir / json_file_name(args.dump)))) + "\n")
            return 0
        return run_check(json_dir, args.verbose)
    except ToolError as error:
        print(f"public-api-lint: error: {error}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
