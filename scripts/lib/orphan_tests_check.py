#!/usr/bin/env python3
"""scripts/lib/orphan_tests_check.py — orphan `tests.rs` / `*_tests.rs` detector.

## Why this exists

AGENTS.md's "unit test は tests.rs に分離する" convention moves
`#[cfg(test)] mod tests { ... }` out of the file it tests and into a sibling
`tests.rs` (or, for a source file with several independently named test
groups, a `tests/` directory of one file per group — see
`crates/raikiri-style/src/property/tests.rs` + `property/tests/*.rs` for the
shape this repo already uses, and `crates/raikiri-dom/src/document/tests.rs`
+ `document/tests/*.rs` for the same shape applied to a single-file crate
module). cargo-llvm-cov's default `--ignore-filename-regex` excludes files
matching this shape from coverage reports (`LLVM_COV_IGNORED_FILENAME_RE`
below, imported from `patch_coverage.py` so the two checkers can never
disagree on what counts) — that exclusion is the entire point of the move.

That exclusion is filename-only, not module-tree-only: cargo-llvm-cov (and
Codecov after it) doesn't care whether the file is actually reachable from
its parent via `mod <name>;`. Neither does rustc, in the failure direction
that matters here — a `.rs` file that exists on disk but is never named by a
`mod` declaration anywhere in its module tree is simply never compiled. It
produces no error and no warning (rustc has no "orphan file" lint; it never
even looks at a file it wasn't told to look at). So a `tests.rs` split that
drops or typos its `mod tests;` line doesn't fail CI on its own merits — its
tests silently stop running (and stop existing as compiled code at all),
while `cargo test`'s pass/fail output and the coverage report both look
unremarkable, because as far as either tool is concerned the tests never
existed. Coverage exclusion working correctly is exactly what makes this
class of mistake invisible to coverage; a drop in test *count* is the only
signal, and nothing today asserts on that count. This checker replaces
that gap with a static, coverage-independent check that every `tests.rs` /
`*_tests.rs` file's expected parent actually names it.

## Method

Purely path- and text-based; this is a lint, not a `rustc`/`syn` parse. For a
candidate file at `<dir>/<X>.rs` matching `LLVM_COV_IGNORED_FILENAME_RE`:

  - if `<dir>` is the crate's `src/` root, the required parent is
    `<dir>/lib.rs` or `<dir>/main.rs` (crate root);
  - otherwise the required parent is `<dir's parent>/<dir's name>.rs` or
    `<dir>/mod.rs` (the two forms Rust's 2018+ module-path resolution
    accepts for a directory submodule).

The parent must contain a `mod <X>;` (or `pub(...) mod <X>;`) declaration,
matched with a start-of-line regex — a `mod` mentioned only in a comment or
string doesn't count as a real declaration, but a real one is never missed
regardless of surrounding formatting or attribute lines above it. This
intentionally does not verify the declaration is `#[cfg(test)]`-guarded: a
`tests.rs` compiled unconditionally into non-test builds is a different,
non-orphan problem the reader would notice immediately as an unexpected
build-time cost, not one that hides silently the way a missing `mod` does.

Nested test *directories* are handled by the same two rules applied
recursively: `property/tests/box_model_tests.rs`'s directory is
`property/tests` (name `tests`), so its required parent is
`property/tests.rs` — the hub file — or `property/tests/mod.rs`. The hub
file `property/tests.rs` is *itself* also a candidate (its own name is
`tests.rs`), whose own required parent (`property.rs` or
`property/mod.rs`) must declare `mod tests;` — both links of the chain are
checked independently, so a hub file that forgets to declare one of its
sub-files, or a hub file that itself isn't reachable, are both caught.

Usage: `python3 orphan_tests_check.py [--repo-root PATH] [-v]`
Exit status: 0 if no orphans found, 1 if at least one is found.
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from patch_coverage import LLVM_COV_IGNORED_FILENAME_RE  # noqa: E402


def _mod_decl_re(name: str) -> re.Pattern[str]:
    return re.compile(r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+" + re.escape(name) + r"\s*;")


@dataclass(frozen=True)
class Orphan:
    path: Path
    mod_name: str
    tried_parents: tuple[Path, ...]


def discover_candidates(repo_root: Path) -> list[Path]:
    """Every `crates/*/src/**/*.rs` file whose name is one cargo-llvm-cov's
    default `--ignore-filename-regex` would exempt from coverage."""
    return sorted(
        p
        for p in repo_root.glob("crates/*/src/**/*.rs")
        if LLVM_COV_IGNORED_FILENAME_RE.search(str(p))
    )


def find_src_root(path: Path) -> Path | None:
    """The nearest ancestor directory literally named `src` — the crate root
    a top-level `<crate>/src/X.rs` is declared from via `lib.rs`/`main.rs`
    rather than a same-named `<crate>/src.rs` sibling (which doesn't exist)."""
    for ancestor in path.parents:
        if ancestor.name == "src":
            return ancestor
    return None


def required_parents(path: Path, src_root: Path) -> tuple[list[Path], str]:
    """(candidate parent files, module name to look for) for `path`."""
    mod_name = path.stem
    parent_dir = path.parent
    if parent_dir == src_root:
        return [parent_dir / "lib.rs", parent_dir / "main.rs"], mod_name
    dir_name = parent_dir.name
    grandparent = parent_dir.parent
    return [grandparent / f"{dir_name}.rs", parent_dir / "mod.rs"], mod_name


def has_mod_decl(parent: Path, mod_name: str) -> bool:
    if not parent.is_file():
        return False
    text = parent.read_text(encoding="utf-8", errors="replace")
    return bool(_mod_decl_re(mod_name).search(text))


def find_orphans(repo_root: Path) -> list[Orphan]:
    orphans = []
    for path in discover_candidates(repo_root):
        src_root = find_src_root(path)
        if src_root is None:
            # Not actually under a `src/` tree — shouldn't happen given the
            # `crates/*/src/**` glob above, but a lint should skip an
            # unexpected layout rather than crash on it.
            continue
        parents, mod_name = required_parents(path, src_root)
        if not any(has_mod_decl(p, mod_name) for p in parents):
            orphans.append(Orphan(path=path, mod_name=mod_name, tried_parents=tuple(parents)))
    return orphans


def _rel(repo_root: Path, path: Path) -> str:
    try:
        return str(path.relative_to(repo_root))
    except ValueError:
        return str(path)


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo-root", default=".", type=Path)
    parser.add_argument("-v", "--verbose", action="store_true", help="list every scanned candidate, not just orphans")
    args = parser.parse_args(argv)
    repo_root = args.repo_root.resolve()

    if args.verbose:
        for path in discover_candidates(repo_root):
            print(f"  scanned: {_rel(repo_root, path)}")

    orphans = find_orphans(repo_root)
    if not orphans:
        print("orphan-tests-lint: PASS (0 orphan tests.rs/*_tests.rs files)")
        return 0

    print(f"orphan-tests-lint: FAIL ({len(orphans)} orphan file(s))")
    for o in orphans:
        tried = " or ".join(_rel(repo_root, p) for p in o.tried_parents)
        print(f"  {_rel(repo_root, o.path)}")
        print(f"    no `mod {o.mod_name};` found in: {tried}")
        print("    this file is outside the module tree: never compiled, its tests never run")
    return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
