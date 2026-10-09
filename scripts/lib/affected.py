#!/usr/bin/env python3
"""scripts/lib/affected.py — select the workspace crates a diff can affect.

## Why this exists

`scripts/gate.sh` builds and tests the whole workspace under several
configurations. That is the right check before a merge, but it is far too
heavy for the edit → test → edit loop, especially when several worktrees
build at once on the same machine. This module narrows the inner loop to the
crates a change can actually influence:

  1. collect changed paths: `git diff --name-only <merge-base>` (committed
     *and* uncommitted changes against the merge-base) plus untracked files;
  2. map each path to the workspace member whose directory contains it;
  3. expand that set with every workspace member that depends on a selected
     member's library. A normal or build dependency changes the dependent's
     library too, so the expansion continues transitively from it. A dev
     dependency only reaches the dependent's tests, benches, and examples, so
     that dependent is selected but the expansion stops there: crates that
     depend on it link its unchanged library.

Paths that change how every crate builds (the root `Cargo.toml`,
`Cargo.lock`, `.cargo/`, `rust-toolchain*`) select the whole workspace.
Paths outside every crate (`scripts/`, `docs/`, `.github/`, ...) select
nothing.

This is a speed tool, not a gate: it does not replace `scripts/gate.sh`.

## Output

One selected package name per line on stdout, sorted. A short decision
trace goes to stderr so the caller can see *why* each crate was selected.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
from pathlib import PurePosixPath

# Root-level paths whose change can alter the build of every workspace
# member. Matched against the repo-relative path (exact file or directory
# prefix).
WORKSPACE_WIDE_FILES = {"Cargo.toml", "Cargo.lock"}
WORKSPACE_WIDE_DIRS = (".cargo/",)
WORKSPACE_WIDE_PREFIXES = ("rust-toolchain",)


def is_workspace_wide(path: str) -> bool:
    if path in WORKSPACE_WIDE_FILES:
        return True
    if path.startswith(WORKSPACE_WIDE_DIRS):
        return True
    return "/" not in path and path.startswith(WORKSPACE_WIDE_PREFIXES)


def member_dirs(metadata: dict, repo_root: str) -> dict[str, str]:
    """Map each workspace member's name to its repo-relative directory."""
    members = set(metadata["workspace_members"])
    root = PurePosixPath(repo_root)
    dirs = {}
    for pkg in metadata["packages"]:
        if pkg["id"] not in members:
            continue
        manifest_dir = PurePosixPath(pkg["manifest_path"]).parent
        dirs[pkg["name"]] = str(manifest_dir.relative_to(root))
    return dirs


def reverse_deps(metadata: dict) -> dict[str, dict[str, bool]]:
    """Map each workspace member to the members that depend on it.

    The inner value maps a dependent's name to whether any of its edges is
    a normal or build dependency (True) rather than only a dev dependency
    (False). Only path dependencies are considered; registry crates are
    never workspace members.
    """
    members = set(metadata["workspace_members"])
    names = {p["name"] for p in metadata["packages"] if p["id"] in members}
    rdeps: dict[str, dict[str, bool]] = {name: {} for name in names}
    for pkg in metadata["packages"]:
        if pkg["id"] not in members:
            continue
        for dep in pkg["dependencies"]:
            if dep.get("path") is None or dep["name"] not in names:
                continue
            if dep["name"] == pkg["name"]:
                continue
            links_lib = dep.get("kind") != "dev"
            dependents = rdeps[dep["name"]]
            dependents[pkg["name"]] = dependents.get(pkg["name"], False) or links_lib
    return rdeps


def owning_member(path: str, dirs: dict[str, str]) -> str | None:
    """Return the member whose directory most specifically contains `path`."""
    best = None
    best_len = -1
    for name, d in dirs.items():
        if d in ("", "."):
            continue
        if (path == d or path.startswith(d + "/")) and len(d) > best_len:
            best, best_len = name, len(d)
    return best


def select(
    changed: list[str],
    metadata: dict,
    repo_root: str,
    *,
    with_rdeps: bool = True,
    trace=None,
) -> set[str]:
    """Return the set of workspace members affected by `changed` paths."""
    log = trace if trace is not None else (lambda _msg: None)
    dirs = member_dirs(metadata, repo_root)
    selected: set[str] = set()
    for path in changed:
        if is_workspace_wide(path):
            log(f"{path}: workspace-wide input -> all members")
            return set(dirs)
        member = owning_member(path, dirs)
        if member is None:
            log(f"{path}: outside every crate -> ignored")
            continue
        if member not in selected:
            log(f"{path}: -> {member}")
        selected.add(member)

    if not with_rdeps:
        return selected

    rdeps = reverse_deps(metadata)
    # Members whose library changed; only these propagate further.
    propagated = set(selected)
    queue = sorted(selected)
    while queue:
        name = queue.pop()
        for dependent, links_lib in sorted(rdeps.get(name, {}).items()):
            if links_lib and dependent not in propagated:
                log(f"{dependent}: depends on {name}")
                selected.add(dependent)
                propagated.add(dependent)
                queue.append(dependent)
            elif not links_lib and dependent not in selected:
                log(f"{dependent}: dev-depends on {name} (tests only)")
                selected.add(dependent)
    return selected


def changed_paths(repo_root: str, base: str) -> list[str]:
    def git(*args: str) -> str:
        return subprocess.run(
            ["git", "-C", repo_root, *args],
            check=True,
            capture_output=True,
            text=True,
        ).stdout

    merge_base = git("merge-base", base, "HEAD").strip()
    tracked = git("diff", "--no-renames", "--name-only", merge_base).splitlines()
    untracked = git("ls-files", "--others", "--exclude-standard").splitlines()
    return sorted({p for p in tracked + untracked if p})


def load_metadata(repo_root: str) -> dict:
    out = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version=1", "--locked"],
        cwd=repo_root,
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return json.loads(out)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repo-root", required=True)
    parser.add_argument("--base", default="main")
    parser.add_argument(
        "--direct",
        action="store_true",
        help="select only the crates containing changed paths, without "
        "reverse dependencies",
    )
    args = parser.parse_args(argv)

    changed = changed_paths(args.repo_root, args.base)
    metadata = load_metadata(args.repo_root)
    selected = select(
        changed,
        metadata,
        args.repo_root,
        with_rdeps=not args.direct,
        trace=lambda msg: print(f"affected: {msg}", file=sys.stderr),
    )
    for name in sorted(selected):
        print(name)
    return 0


if __name__ == "__main__":
    sys.exit(main())
