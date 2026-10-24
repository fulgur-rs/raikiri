#!/usr/bin/env bash
# scripts/taffy-boundary-lint.sh — Taffy boundary ratchet.
#
# Wraps scripts/lib/taffy_boundary_lint.py — see that module's docstring for
# what is counted, how the per-file baseline works, and how to mark a real
# exception. In short: Taffy's types may only be named by the adapter and the
# view crate. Layout results, geometry, the layout call types and the node
# handle have owned names that must be used everywhere else; the style input
# (traits, lengths, keywords, Style, float and grid state) has none yet, so no
# new uses may be added. While the migration to owned names is under way, the
# number of remaining uses is pinned per file in
# scripts/lib/taffy_boundary_baseline.txt and may only go down.
#
# Usage:
#   scripts/taffy-boundary-lint.sh                    # gate mode: PASS/FAIL
#   scripts/taffy-boundary-lint.sh -v                 # also lists every occurrence
#   scripts/taffy-boundary-lint.sh --print-counts     # per-file counts, baseline format
#   scripts/taffy-boundary-lint.sh --update-baseline  # lower the baseline (never raises a line)
#
# Exit status: 0 if nothing new and the baseline is tight, 1 on a ratchet
# failure, 2 on a tooling error (unreadable baseline or source).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=lib/repo_root.sh
source "$SCRIPT_DIR/lib/repo_root.sh"

exec python3 "$SCRIPT_DIR/lib/taffy_boundary_lint.py" --repo-root "$REPO_ROOT" "$@"
