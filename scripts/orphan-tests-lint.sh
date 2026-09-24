#!/usr/bin/env bash
# scripts/orphan-tests-lint.sh — orphan `tests.rs` / `*_tests.rs` scan.
#
# Wraps scripts/lib/orphan_tests_check.py — see that module's docstring for
# the full rationale (why a `tests.rs` split can drop its tests silently,
# with no compiler warning and no coverage signal) and the exact detection
# method.
#
# Usage:
#   scripts/orphan-tests-lint.sh       # gate mode: prints scan result, PASS/FAIL
#   scripts/orphan-tests-lint.sh -v    # also lists every scanned candidate
#
# Exit status: 0 if every tests.rs/*_tests.rs file is reachable from its
# expected parent, 1 if at least one is not.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=lib/repo_root.sh
source "$SCRIPT_DIR/lib/repo_root.sh"

exec python3 "$SCRIPT_DIR/lib/orphan_tests_check.py" --repo-root "$REPO_ROOT" "$@"
