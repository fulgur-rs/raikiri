#!/usr/bin/env bash
# scripts/public-api-lint.sh — no Taffy type in the public API of the crates
# that other code builds on, and no type of an unlisted crate in the public API
# of a published crate.
#
# Wraps scripts/lib/public_api_lint.py — see that module's docstring for how the
# public API is read (rustdoc JSON, hidden items included), which crates are
# checked and why the style view crate is not held to it. In short: no
# parameter, return type, field, bound, trait impl or re-export of a public
# item of raikiri-dom, raikiri-html, raikiri-paint, raikiri-traits, raikiri or
# raikiri-layout-geometry may name a type of the `taffy` crate, and a published
# crate may name only the types of other crates that THIRD_PARTY_ALLOWED lists
# for it (docs/public-api.md has the policy).
#
# Usage:
#   scripts/public-api-lint.sh                  # gate mode: PASS/FAIL (runs cargo doc, ~10 s warm)
#   scripts/public-api-lint.sh -v               # also counts the public items per crate
#   scripts/public-api-lint.sh --dump CRATE     # prints the public API of CRATE, one line per item
#   scripts/public-api-lint.sh --surface CRATE  # prints the names CRATE publishes (add --with-hidden for hidden ones)
#   scripts/public-api-lint.sh --exhaustive CRATE  # prints the types of CRATE without #[non_exhaustive]
#   scripts/public-api-lint.sh --usage DIR      # reports which published names the Rust sources under DIR reach
#   scripts/public-api-lint.sh --json-dir DIR   # reads the rustdoc JSON from DIR instead of running cargo
#
# To check that a change left the public signatures of a crate alone, dump it
# in a checkout of the base and in the head and diff the two outputs.
#
# Run it after the `cargo doc --workspace` step: the JSON comes from the same
# dependency build, started with rustdoc alone allowed to use unstable flags.
#
# Exit status: 0 if no checked item names Taffy and no published crate names an
# unlisted crate, 1 if some does, 2 on a tooling error (cargo failed, a JSON file is missing or of an unknown format).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=lib/repo_root.sh
source "$SCRIPT_DIR/lib/repo_root.sh"

cd "$REPO_ROOT"
# cargo doc and the scratch directory of the lint go under the pinned TMPDIR,
# as for the other gate scripts.
# shellcheck source=lib/tmpdir.sh
source "$SCRIPT_DIR/lib/tmpdir.sh"

exec python3 "$SCRIPT_DIR/lib/public_api_lint.py" --repo-root "$REPO_ROOT" "$@"
