#!/usr/bin/env python3
"""scripts/lib/publish_dry_run.py — which workspace crates can be published.

`scripts/publish-dry-run.sh` packages the crates that are meant for crates.io
and keeps the list in one place. This module checks that list against the
manifests, so a crate that gains or loses `publish = false` cannot slip through
unnoticed, and a new crate has to be listed (or marked private) on purpose.

Usage:

    cargo metadata --no-deps --locked --format-version 1 \\
      | python3 scripts/lib/publish_dry_run.py check CRATE [CRATE...]

Exit status: 0 when the workspace members that may be published are exactly
CRATE..., 1 when they differ (the difference is printed), 2 on bad input.
"""

import json
import sys


def publishable_members(metadata):
    """Names of the workspace members whose manifest allows publishing.

    `cargo metadata` reports `publish = false` as an empty list, `publish =
    true` (the default) as null and `publish = ["registry"]` as that list, so
    a crate restricted to other registries is not published to crates.io.
    """
    members = set(metadata["workspace_members"])
    return sorted(
        package["name"]
        for package in metadata["packages"]
        if package["id"] in members and publishes_to_crates_io(package)
    )


def publishes_to_crates_io(package):
    registries = package.get("publish")
    return registries is None or "crates-io" in registries


def compare(metadata, expected):
    """Error messages for the difference between the manifests and `expected`."""
    actual = set(publishable_members(metadata))
    wanted = set(expected)
    errors = []
    for name in sorted(actual - wanted):
        errors.append(
            f"{name} may be published (no `publish = false`) but is not in the "
            "list of crates to publish; list it, or set `publish = false`"
        )
    for name in sorted(wanted - actual):
        errors.append(
            f"{name} is in the list of crates to publish but is not a "
            "publishable workspace member"
        )
    return errors


def main(argv):
    if len(argv) < 3 or argv[1] != "check":
        print(__doc__, file=sys.stderr)
        return 2
    try:
        metadata = json.load(sys.stdin)
    except json.JSONDecodeError as error:
        print(f"publish_dry_run.py: cargo metadata is not JSON: {error}", file=sys.stderr)
        return 2
    errors = compare(metadata, argv[2:])
    for error in errors:
        print(f"publish-dry-run: {error}", file=sys.stderr)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
